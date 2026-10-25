//! Glass: the two system values librespot reads through sysinfo, which doesn't
//! build for iOS.

/// A string sysctl value, such as `kern.osproductversion` ("27.0").
pub(crate) fn sysctl_string(name: &str) -> Option<String> {
    let name = std::ffi::CString::new(name).ok()?;
    let mut len: libc::size_t = 0;
    // SAFETY: a size query (null buffer), then a read into a buffer of that size.
    unsafe {
        if libc::sysctlbyname(name.as_ptr(), std::ptr::null_mut(), &mut len, std::ptr::null_mut(), 0) != 0 || len == 0 {
            return None;
        }
        let mut buf = vec![0u8; len];
        if libc::sysctlbyname(name.as_ptr(), buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) != 0 {
            return None;
        }
        buf.truncate(len);
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8(buf[..end].to_vec()).ok()
    }
}
