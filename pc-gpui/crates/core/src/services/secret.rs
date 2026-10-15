//! 本地敏感配置（WebDAV 密码）的落盘保护（A12）。
//!
//! - Windows：DPAPI（`CryptProtectData` / `CryptUnprotectData`，当前用户作用域），存储格式
//!   `dpapi:<base64>`。密文只有同一 Windows 用户在本机能解开，拷走 `data.db` 拿不到明文。
//! - 其它平台：没有系统级等价物可用（Keychain / Secret Service 需要额外依赖与授权交互），
//!   按原样存明文。PC 端目标平台是 Windows，其它平台只是能跑。
//! - 兼容旧数据：没有 `dpapi:` 前缀的值视为旧版明文，读取时直接使用，下次保存设置时加密。

use base64::{engine::general_purpose::STANDARD, Engine as _};

/// DPAPI 密文前缀
const DPAPI_PREFIX: &str = "dpapi:";

/// 存储值是否已经是受保护的形式
pub fn is_protected(stored: &str) -> bool {
    stored.starts_with(DPAPI_PREFIX)
}

/// 把明文转换为存储形式。空串原样返回（表示"没有密码"）。
pub fn protect(plain: &str) -> Result<String, String> {
    if plain.is_empty() {
        return Ok(String::new());
    }
    platform::protect(plain.as_bytes())
        .map(|blob| format!("{}{}", DPAPI_PREFIX, STANDARD.encode(blob)))
        .or_else(|e| platform::fallback_plain(plain, e))
}

/// 把存储形式还原为明文。旧版明文原样返回。
pub fn reveal(stored: &str) -> Result<String, String> {
    let Some(encoded) = stored.strip_prefix(DPAPI_PREFIX) else {
        return Ok(stored.to_string());
    };
    let blob = STANDARD
        .decode(encoded)
        .map_err(|e| format!("已保存的密码格式损坏: {}", e))?;
    let plain = platform::unprotect(&blob)?;
    String::from_utf8(plain).map_err(|_| "已保存的密码格式损坏".to_string())
}

#[cfg(windows)]
mod platform {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    /// 附加熵：把密文绑定到本应用，其它程序即使以同一用户身份调用 DPAPI 也需要知道它
    const ENTROPY: &[u8] = b"mini-todo/webdav-password/v1";

    fn blob_of(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            // DPAPI 的入参不会被写入，*mut 只是 C 头文件的声明形式
            pbData: data.as_ptr() as *mut u8,
        }
    }

    /// 把 DPAPI 分配的输出缓冲区拷贝出来并用 `LocalFree` 释放
    unsafe fn take_output(out: &mut CRYPT_INTEGER_BLOB) -> Vec<u8> {
        if out.pbData.is_null() {
            return Vec::new();
        }
        let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(HLOCAL(out.pbData as *mut core::ffi::c_void));
        out.pbData = std::ptr::null_mut();
        out.cbData = 0;
        bytes
    }

    pub fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob_of(plain);
        let entropy = blob_of(ENTROPY);
        let mut output = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptProtectData(
                &input,
                PCWSTR::null(),
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
            .map_err(|e| format!("加密密码失败: {}", e))?;
            Ok(take_output(&mut output))
        }
    }

    pub fn unprotect(blob: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob_of(blob);
        let entropy = blob_of(ENTROPY);
        let mut output = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptUnprotectData(
                &input,
                None,
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
            .map_err(|e| format!("无法解密已保存的密码（可能来自其它电脑或其它用户）: {}", e))?;
            Ok(take_output(&mut output))
        }
    }

    /// Windows 上加密失败就报错，不静默退回明文
    pub fn fallback_plain(_plain: &str, err: String) -> Result<String, String> {
        Err(err)
    }
}

#[cfg(not(windows))]
mod platform {
    //! 非 Windows：不加密（见模块文档）。`protect` 总是"失败"，由 `fallback_plain` 退回明文存储。

    pub fn protect(_plain: &[u8]) -> Result<Vec<u8>, String> {
        Err("当前平台不支持 DPAPI".to_string())
    }

    pub fn unprotect(_blob: &[u8]) -> Result<Vec<u8>, String> {
        Err("该密码由 Windows 加密保存，当前平台无法解密，请重新输入".to_string())
    }

    /// 非 Windows 平台明文存储
    pub fn fallback_plain(plain: &str, _err: String) -> Result<String, String> {
        Ok(plain.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_password_stays_empty() {
        assert_eq!(protect("").unwrap(), "");
        assert_eq!(reveal("").unwrap(), "");
    }

    #[test]
    fn legacy_plaintext_is_readable() {
        assert!(!is_protected("hunter2"));
        assert_eq!(reveal("hunter2").unwrap(), "hunter2");
    }

    #[test]
    fn protect_reveal_round_trip() {
        let stored = protect("p@ss 中文").unwrap();
        assert_eq!(reveal(&stored).unwrap(), "p@ss 中文");
        #[cfg(windows)]
        {
            assert!(is_protected(&stored));
            assert!(!stored.contains("p@ss"));
        }
        #[cfg(not(windows))]
        assert_eq!(stored, "p@ss 中文");
    }

    #[test]
    fn corrupted_ciphertext_is_an_error() {
        assert!(reveal("dpapi:!!!not-base64!!!").is_err());
        #[cfg(not(windows))]
        assert!(reveal("dpapi:AAAA").is_err());
    }
}
