//! Rewrites an AAC MP4 as a plain M4A: one `mdat` with the samples in order, then a
//! `moov` with a full sample table and (optionally) iTunes tags. The audio is copied as
//! it is, streaming, so memory stays small whatever the file size.
//!
//! Sources are YouTube's fragmented DASH audio (`moof`/`mdat` pairs) and JioSaavn's plain
//! MP4s. Only the boxes a single audio track needs are read. Every size and count in the
//! file is checked against the file's length before it's used, so a broken download fails
//! with an error rather than a huge allocation.

use super::Song;
use anyhow::{Context, Result, bail, ensure};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

struct BoxHeader {
    kind: [u8; 4],
    start: u64,
    size: u64,
    header: u64,
}

fn headers(r: &mut (impl Read + Seek), from: u64, to: u64) -> Result<Vec<BoxHeader>> {
    let mut out = Vec::new();
    let mut pos = from;
    while pos + 8 <= to {
        r.seek(SeekFrom::Start(pos))?;
        let mut h = [0u8; 8];
        r.read_exact(&mut h)?;
        let mut size = u32::from_be_bytes(h[..4].try_into().unwrap()) as u64;
        let mut header = 8;
        if size == 1 {
            let mut big = [0u8; 8];
            r.read_exact(&mut big)?;
            size = u64::from_be_bytes(big);
            header = 16;
        } else if size == 0 {
            size = to - pos;
        }
        let end = pos.checked_add(size).filter(|&end| size >= header && end <= to).with_context(|| format!("broken MP4 box at {pos}"))?;
        out.push(BoxHeader { kind: h[4..8].try_into().unwrap(), start: pos, size, header });
        pos = end;
    }
    Ok(out)
}

/// Children of a box held in memory, as (type, body).
fn children(buf: &[u8]) -> Result<Vec<([u8; 4], &[u8])>> {
    let mut c = std::io::Cursor::new(buf);
    Ok(headers(&mut c, 0, buf.len() as u64)?.into_iter().map(|h| (h.kind, &buf[(h.start + h.header) as usize..(h.start + h.size) as usize])).collect())
}

fn child<'a>(buf: &'a [u8], path: &[&[u8; 4]]) -> Result<&'a [u8]> {
    let mut cur = buf;
    for kind in path {
        cur = children(cur)?.into_iter().find(|(k, _)| k == *kind).map(|(_, b)| b).with_context(|| format!("MP4 without {}", String::from_utf8_lossy(*kind)))?;
    }
    Ok(cur)
}

fn be32(b: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(b.get(at..at + 4).context("short MP4 box")?.try_into().unwrap()))
}

fn be64(b: &[u8], at: usize) -> Result<u64> {
    Ok(u64::from_be_bytes(b.get(at..at + 8).context("short MP4 box")?.try_into().unwrap()))
}

/// One sample's place in the source file.
struct Sample {
    offset: u64,
    size: u32,
    duration: u32,
}

/// What the rewrite needs from the source.
struct Track {
    samples: Vec<Sample>,
    timescale: u32,
    /// The `stsd` body (codec setup), copied as it is.
    stsd: Vec<u8>,
}

fn read_box(input: &mut (impl Read + Seek), h: &BoxHeader) -> Result<Vec<u8>> {
    let mut body = vec![0u8; (h.size - h.header) as usize];
    input.seek(SeekFrom::Start(h.start + h.header))?;
    input.read_exact(&mut body)?;
    Ok(body)
}

fn read_track(input: &mut (impl Read + Seek), len: u64) -> Result<Track> {
    let top = headers(input, 0, len)?;
    let moov = read_box(input, top.iter().find(|h| &h.kind == b"moov").context("MP4 without moov")?)?;
    let mdia = child(&moov, &[b"trak", b"mdia"])?;
    let mdhd = child(mdia, &[b"mdhd"])?;
    let timescale = match mdhd.first() {
        Some(1) => be32(mdhd, 20)?,
        Some(_) => be32(mdhd, 12)?,
        None => bail!("short MP4 box"),
    };
    let stbl = child(mdia, &[b"minf", b"stbl"])?;
    let stsd = child(stbl, &[b"stsd"])?.to_vec();

    let fragments: Vec<&BoxHeader> = top.iter().filter(|h| &h.kind == b"moof").collect();
    let samples = if fragments.is_empty() {
        plain_samples(stbl, len)?
    } else {
        let defaults = match child(&moov, &[b"mvex", b"trex"]) {
            Ok(t) => (be32(t, 12)?, be32(t, 16)?),
            Err(_) => (0, 0),
        };
        let mut samples = Vec::new();
        for moof in fragments {
            fragment_samples(&read_box(input, moof)?, moof.start, defaults, len, &mut samples)?;
        }
        samples
    };
    ensure!(!samples.is_empty(), "MP4 without audio samples");
    ensure!(samples.iter().all(|s| s.offset.checked_add(s.size as u64).is_some_and(|end| end <= len)), "MP4 is cut short");
    Ok(Track { samples, timescale, stsd })
}

/// Samples of a plain MP4 (`len` bytes), from its sample table.
fn plain_samples(stbl: &[u8], len: u64) -> Result<Vec<Sample>> {
    let stsz = child(stbl, &[b"stsz"])?;
    let fixed = be32(stsz, 4)?;
    let count = be32(stsz, 8)? as usize;
    // Every sample takes at least a byte of the file, and listed sizes must be in the box.
    let fits = if fixed != 0 { (count as u64).checked_mul(fixed as u64).is_some_and(|n| n <= len) } else { stsz.len() >= 12 + 4 * count };
    ensure!(fits, "MP4 sample table doesn't fit the file");
    let sizes: Vec<u32> = (0..count).map(|i| if fixed != 0 { Ok(fixed) } else { be32(stsz, 12 + 4 * i) }).collect::<Result<_>>()?;

    let mut durations = Vec::with_capacity(count);
    let stts = child(stbl, &[b"stts"])?;
    for e in 0..be32(stts, 4)? as usize {
        if durations.len() >= count {
            break;
        }
        let (n, delta) = (be32(stts, 8 + 8 * e)?, be32(stts, 12 + 8 * e)?);
        durations.extend(std::iter::repeat_n(delta, (n as usize).min(count - durations.len())));
    }
    ensure!(durations.len() >= count, "MP4 sample durations don't add up");

    let chunks: Vec<u64> = match child(stbl, &[b"stco"]) {
        Ok(stco) => (0..be32(stco, 4)? as usize).map(|i| be32(stco, 8 + 4 * i).map(u64::from)).collect::<Result<_>>()?,
        Err(_) => {
            let co64 = child(stbl, &[b"co64"])?;
            (0..be32(co64, 4)? as usize).map(|i| be64(co64, 8 + 8 * i)).collect::<Result<_>>()?
        }
    };
    let stsc = child(stbl, &[b"stsc"])?;
    let runs: Vec<(u32, u32)> = (0..be32(stsc, 4)? as usize).map(|e| Ok((be32(stsc, 8 + 12 * e)?, be32(stsc, 12 + 12 * e)?))).collect::<Result<_>>()?;

    let mut samples = Vec::with_capacity(count);
    for (c, &chunk_offset) in chunks.iter().enumerate() {
        let chunk = c as u32 + 1;
        let per_chunk = runs.iter().rev().find(|(first, _)| *first <= chunk).map(|r| r.1).context("MP4 chunk without samples")?;
        let mut offset = chunk_offset;
        for _ in 0..per_chunk {
            let i = samples.len();
            if i == count {
                break;
            }
            samples.push(Sample { offset, size: sizes[i], duration: durations[i] });
            offset = offset.checked_add(sizes[i] as u64).context("bad MP4 chunk offset")?;
        }
    }
    ensure!(samples.len() == count, "MP4 sample table doesn't add up");
    Ok(samples)
}

/// Samples of one `moof` (body), whose box starts at `moof_start` in a file of `len` bytes.
fn fragment_samples(moof: &[u8], moof_start: u64, (trex_duration, trex_size): (u32, u32), len: u64, out: &mut Vec<Sample>) -> Result<()> {
    for (kind, traf) in children(moof)? {
        if &kind != b"traf" {
            continue;
        }
        let tfhd = child(traf, &[b"tfhd"])?;
        let flags = be32(tfhd, 0)? & 0xFF_FFFF;
        let mut at = 8; // version/flags, track_ID
        let mut base = moof_start;
        if flags & 0x01 != 0 {
            base = be64(tfhd, at)?;
            at += 8;
        }
        if flags & 0x02 != 0 {
            at += 4;
        }
        let mut default_duration = trex_duration;
        if flags & 0x08 != 0 {
            default_duration = be32(tfhd, at)?;
            at += 4;
        }
        let default_size = if flags & 0x10 != 0 { be32(tfhd, at)? } else { trex_size };
        for (kind, trun) in children(traf)? {
            if &kind != b"trun" {
                continue;
            }
            let flags = be32(trun, 0)? & 0xFF_FFFF;
            let count = be32(trun, 4)?;
            let mut at = 8;
            let mut offset = base;
            if flags & 0x01 != 0 {
                offset = base.checked_add_signed(be32(trun, at)? as i32 as i64).context("bad MP4 data offset")?;
                at += 4;
            }
            if flags & 0x04 != 0 {
                at += 4;
            }
            // The count must fit what's there: the per-sample fields in this box, or the
            // file itself when every sample has the default size. And all fragments
            // together can't have more samples than the file has bytes.
            let per_sample = 4 * [0x100, 0x200, 0x400, 0x800].iter().filter(|f| flags & **f != 0).count();
            let fits = match trun.len().saturating_sub(at).checked_div(per_sample) {
                Some(room) => room >= count as usize,
                None => (count as u64).saturating_mul(default_size.max(1) as u64) <= len,
            };
            ensure!(fits && out.len() as u64 + count as u64 <= len, "MP4 fragment doesn't fit the file");
            for _ in 0..count {
                let mut duration = default_duration;
                let mut size = default_size;
                if flags & 0x100 != 0 {
                    duration = be32(trun, at)?;
                    at += 4;
                }
                if flags & 0x200 != 0 {
                    size = be32(trun, at)?;
                    at += 4;
                }
                at += 4 * ((flags & 0x400 != 0) as usize + (flags & 0x800 != 0) as usize);
                out.push(Sample { offset, size, duration });
                offset = offset.checked_add(size as u64).context("bad MP4 data offset")?;
            }
        }
    }
    Ok(())
}

/// Writes `src`'s audio to `dst` as a plain M4A, with `tags` (the song and its cover) if given.
pub fn rewrite(src: &Path, dst: &Path, tags: Option<(&Song, Option<&[u8]>)>) -> Result<()> {
    let mut input = BufReader::new(File::open(src)?);
    let len = input.get_ref().metadata()?.len();
    let track = read_track(&mut input, len)?;
    let data_len: u64 = track.samples.iter().map(|s| s.size as u64).sum();
    if data_len == 0 {
        bail!("no audio to save");
    }
    // Samples that overlap could add up to far more than the file.
    ensure!(data_len <= len, "MP4 sample table doesn't fit the file");
    let duration: u64 = track.samples.iter().map(|s| s.duration as u64).sum();
    let mut out = BufWriter::new(File::create(dst)?);

    // ftyp, mdat, then moov: with moov last, nothing moves if the tags change later.
    let ftyp = boxed(b"ftyp", &[b"M4A ".as_slice(), &0u32.to_be_bytes(), b"M4A ", b"mp42", b"isom"].concat());
    out.write_all(&ftyp)?;
    let large = data_len + 8 > u32::MAX as u64;
    let data_start = ftyp.len() as u64 + if large { 16 } else { 8 };
    if large {
        out.write_all(&1u32.to_be_bytes())?;
        out.write_all(b"mdat")?;
        out.write_all(&(data_len + 16).to_be_bytes())?;
    } else {
        out.write_all(&((data_len + 8) as u32).to_be_bytes())?;
        out.write_all(b"mdat")?;
    }
    let mut buf = Vec::new();
    let mut pos = u64::MAX;
    for s in &track.samples {
        buf.resize(s.size as usize, 0);
        // Samples mostly run back to back; seek only across gaps.
        if s.offset != pos {
            input.seek(SeekFrom::Start(s.offset))?;
        }
        input.read_exact(&mut buf)?;
        out.write_all(&buf)?;
        pos = s.offset + s.size as u64;
    }
    let udta = tags.map(|(song, cover)| udta(song, cover)).unwrap_or_default();
    out.write_all(&moov(&track, duration, data_start, &udta))?;
    out.flush()?;
    Ok(())
}

fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(body.len() + 8);
    b.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
    b.extend_from_slice(kind);
    b.extend_from_slice(body);
    b
}

/// A full box: version/flags, then the body.
fn full(kind: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let mut b = vec![version];
    b.extend_from_slice(&flags.to_be_bytes()[1..]);
    b.extend_from_slice(body);
    boxed(kind, &b)
}

fn u32s(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_be_bytes()).collect()
}

const MATRIX: [u32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];

fn moov(track: &Track, duration: u64, data_start: u64, udta: &[u8]) -> Vec<u8> {
    let (samples, timescale) = (&track.samples, track.timescale);
    let dur = duration.min(u32::MAX as u64) as u32;

    let mut mvhd = u32s(&[0, 0, timescale, dur, 0x0001_0000]);
    mvhd.extend_from_slice(&0x0100u16.to_be_bytes());
    mvhd.extend_from_slice(&[0; 10]);
    mvhd.extend(u32s(&MATRIX));
    mvhd.extend_from_slice(&[0; 24]);
    mvhd.extend(u32s(&[2]));

    let mut tkhd = u32s(&[0, 0, 1, 0, dur, 0, 0]);
    tkhd.extend_from_slice(&[0, 0, 0, 0]); // layer, alternate group
    tkhd.extend_from_slice(&0x0100u16.to_be_bytes());
    tkhd.extend_from_slice(&[0, 0]);
    tkhd.extend(u32s(&MATRIX));
    tkhd.extend(u32s(&[0, 0]));

    let mut mdhd = u32s(&[0, 0, timescale, dur]);
    mdhd.extend_from_slice(&0x55C4u16.to_be_bytes()); // "und"
    mdhd.extend_from_slice(&[0, 0]);

    let mut hdlr = u32s(&[0]);
    hdlr.extend_from_slice(b"soun");
    hdlr.extend_from_slice(&[0; 12]);
    hdlr.extend_from_slice(b"SoundHandler\0");

    // Durations, run-length encoded.
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for s in samples {
        match runs.last_mut() {
            Some((count, delta)) if *delta == s.duration => *count += 1,
            _ => runs.push((1, s.duration)),
        }
    }
    let mut stts = u32s(&[runs.len() as u32]);
    stts.extend(u32s(&runs.iter().flat_map(|&(c, d)| [c, d]).collect::<Vec<_>>()));
    let mut stsz = u32s(&[0, samples.len() as u32]);
    stsz.extend(u32s(&samples.iter().map(|s| s.size).collect::<Vec<_>>()));
    // Every sample in one chunk.
    let stsc = u32s(&[1, 1, samples.len() as u32, 1]);
    let chunk_offset =
        if data_start > u32::MAX as u64 { full(b"co64", 0, 0, &[1u32.to_be_bytes().as_slice(), &data_start.to_be_bytes()].concat()) } else { full(b"stco", 0, 0, &u32s(&[1, data_start as u32])) };

    let stbl = boxed(b"stbl", &[boxed(b"stsd", &track.stsd), full(b"stts", 0, 0, &stts), full(b"stsc", 0, 0, &stsc), full(b"stsz", 0, 0, &stsz), chunk_offset].concat());
    let dinf = boxed(b"dinf", &full(b"dref", 0, 0, &[u32s(&[1]), full(b"url ", 0, 1, &[])].concat()));
    let minf = boxed(b"minf", &[full(b"smhd", 0, 0, &[0; 4]), dinf, stbl].concat());
    let mdia = boxed(b"mdia", &[full(b"mdhd", 0, 0, &mdhd), full(b"hdlr", 0, 0, &hdlr), minf].concat());
    let trak = boxed(b"trak", &[full(b"tkhd", 0, 3, &tkhd), mdia].concat());
    boxed(b"moov", &[full(b"mvhd", 0, 0, &mvhd), trak, udta.to_vec()].concat())
}

/// iTunes-style tags: `udta/meta/ilst`.
fn udta(song: &Song, cover: Option<&[u8]>) -> Vec<u8> {
    // An item holds one `data` box: type (1 = UTF-8, 13 = JPEG, 14 = PNG, 0 = binary), locale, value.
    let item = |kind: &[u8; 4], data_type: u32, value: &[u8]| boxed(kind, &boxed(b"data", &[u32s(&[data_type, 0]), value.to_vec()].concat()));
    let text = |kind: &[u8; 4], value: &str| item(kind, 1, value.as_bytes());
    let mut ilst = Vec::new();
    ilst.extend(text(b"\xA9nam", &song.title));
    ilst.extend(text(b"\xA9ART", &song.artists.join(", ")));
    if let Some(album) = &song.album {
        ilst.extend(text(b"\xA9alb", album));
    }
    if !song.album_artists.is_empty() {
        ilst.extend(text(b"aART", &song.album_artists.join(", ")));
    }
    if let Some(year) = &song.year {
        ilst.extend(text(b"\xA9day", year));
    }
    if let Some(n) = song.track_number {
        // Reserved, track, total, reserved.
        ilst.extend(item(b"trkn", 0, &[0, 0, (n >> 8) as u8, n as u8, 0, 0, 0, 0]));
    }
    if let Some(link) = &song.link {
        ilst.extend(text(b"\xA9cmt", link));
    }
    if let Some(data) = cover.filter(|c| !c.is_empty()) {
        let png = data.starts_with(&[0x89, b'P', b'N', b'G']);
        ilst.extend(item(b"covr", if png { 14 } else { 13 }, data));
    }
    let mut hdlr = u32s(&[0]);
    hdlr.extend_from_slice(b"mdir");
    hdlr.extend_from_slice(b"appl");
    hdlr.extend_from_slice(&[0; 9]);
    let meta = full(b"meta", 0, 0, &[full(b"hdlr", 0, 0, &hdlr), boxed(b"ilst", &ilst)].concat());
    boxed(b"udta", &meta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broken_sizes_fail_instead_of_allocating() {
        // A 64-bit box size that would wrap the read position back to the start.
        let mut b = Vec::new();
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(b"free");
        b.extend_from_slice(&(u64::MAX - 7).to_be_bytes());
        b.extend_from_slice(&[0u8; 16]);
        assert!(headers(&mut std::io::Cursor::new(&b), 0, b.len() as u64).is_err());
        // A sample table claiming four billion one-byte samples in a small file.
        let stsz = boxed(b"stsz", &[0u32.to_be_bytes(), 1u32.to_be_bytes(), u32::MAX.to_be_bytes()].concat());
        assert!(plain_samples(&stsz, 1_000).is_err());
        // Fragments that each fit a 1000-byte file, but not all together.
        let traf = boxed(b"traf", &[full(b"tfhd", 0, 0x10, &u32s(&[1, 1])), full(b"trun", 0, 0, &u32s(&[600]))].concat());
        let mut samples = Vec::new();
        fragment_samples(&traf, 0, (1024, 0), 1_000, &mut samples).unwrap();
        assert_eq!(samples.len(), 600);
        assert!(fragment_samples(&traf, 0, (1024, 0), 1_000, &mut samples).is_err());
    }

    /// A tiny plain MP4 (3 samples in 2 chunks) rewritten with tags reads back with lofty,
    /// the way Local Files will read it.
    #[test]
    fn plain_mp4s_are_rewritten_with_tags() {
        let dir = crate::config::test_dir("mp4");
        std::fs::create_dir_all(&dir).unwrap();
        let (src, dst) = (dir.join("in.mp4"), dir.join("out.m4a"));
        let samples: [&[u8]; 3] = [b"aaaa", b"bb", b"cccccc"];
        // AAC-LC, 44.1 kHz, stereo.
        let esds = full(b"esds", 0, 0, &[3, 25, 0, 1, 0, 4, 17, 0x40, 0x15, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 5, 2, 0x12, 0x10, 6, 1, 2]);
        let mut mp4a = vec![0; 6];
        mp4a.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 16, 0, 0, 0, 0, 0xAC, 0x44, 0, 0]);
        mp4a.extend(esds);
        let stsd = [u32s(&[0, 1]), boxed(b"mp4a", &mp4a)].concat();
        let ftyp = boxed(b"ftyp", b"isom\0\0\0\0isom");
        let mdat = boxed(b"mdat", &samples.concat());
        let first = ftyp.len() as u32 + 8;
        let stbl = boxed(
            b"stbl",
            &[
                boxed(b"stsd", &stsd),
                full(b"stts", 0, 0, &u32s(&[1, 3, 1024])),
                full(b"stsc", 0, 0, &u32s(&[2, 1, 2, 1, 2, 1, 1])),
                full(b"stsz", 0, 0, &u32s(&[0, 3, 4, 2, 6])),
                full(b"stco", 0, 0, &u32s(&[2, first, first + 6])),
            ]
            .concat(),
        );
        let mdhd = [u32s(&[0, 0, 44100, 3072]), vec![0x55, 0xC4, 0, 0]].concat();
        let mdia = boxed(b"mdia", &[full(b"mdhd", 0, 0, &mdhd), boxed(b"minf", &stbl)].concat());
        std::fs::write(&src, [ftyp, mdat, boxed(b"moov", &boxed(b"trak", &mdia))].concat()).unwrap();

        let song = Song {
            title: "Kesariya".into(),
            artists: vec!["Pritam".into(), "Arijit Singh".into()],
            album: Some("Brahmastra".into()),
            track_number: Some(1),
            year: Some("2022".into()),
            ..Default::default()
        };
        rewrite(&src, &dst, Some((&song, Some(&[0xFF, 0xD8, 0xFF, 0xE0])))).unwrap();

        let out = std::fs::read(&dst).unwrap();
        assert_eq!(&out[28 + 8..28 + 8 + 12], b"aaaabbcccccc", "the samples, in order");
        use lofty::prelude::*;
        let file = lofty::probe::Probe::open(&dst).unwrap().guess_file_type().unwrap().read().unwrap();
        let tag = file.primary_tag().expect("tags");
        assert_eq!(tag.title().as_deref(), Some("Kesariya"));
        assert_eq!(tag.artist().as_deref(), Some("Pritam, Arijit Singh"));
        assert_eq!(tag.album().as_deref(), Some("Brahmastra"));
        assert_eq!(tag.track(), Some(1));
        assert_eq!(tag.pictures().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
