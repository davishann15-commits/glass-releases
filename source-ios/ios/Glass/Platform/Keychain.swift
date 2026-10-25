import Foundation
import Security

/// The Keychain for the core's secrets (Spotify's refresh token, the assistant's
/// tokens and API key). Generic passwords for this device only, readable after
/// the first unlock: a token can then be refreshed while the phone is locked
/// during background playback, but never leaves the device in a backup.
final class KeychainStore: Keychain, @unchecked Sendable {
    private let service = "app.glass.ios"

    private func query(_ key: String) -> [String: Any] {
        [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service, kSecAttrAccount as String: key]
    }

    func get(key: String) -> String? {
        var q = query(key)
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var out: CFTypeRef?
        guard SecItemCopyMatching(q as CFDictionary, &out) == errSecSuccess, let data = out as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }

    func set(key: String, value: String) -> Bool {
        let data = Data(value.utf8)
        let attrs: [String: Any] = [kSecValueData as String: data, kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly]
        let status = SecItemUpdate(query(key) as CFDictionary, attrs as CFDictionary)
        if status == errSecSuccess { return true }
        guard status == errSecItemNotFound else { return false }
        var add = query(key)
        add.merge(attrs) { $1 }
        return SecItemAdd(add as CFDictionary, nil) == errSecSuccess
    }

    func delete(key: String) {
        SecItemDelete(query(key) as CFDictionary)
    }
}
