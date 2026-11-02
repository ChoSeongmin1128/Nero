pub(crate) const PRIVACY_POLICY: &str = "https://choseongmin1128.github.io/Nero/privacy.html";

pub(crate) fn open_url(url: &reqwest::Url) -> Result<(), ()> {
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(());
    }
    let url = url.as_str();
    #[cfg(target_os = "macos")]
    if std::process::Command::new("/usr/bin/open")
        .arg(url)
        .status()
        .is_ok_and(|status| status.success())
    {
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "shell32")]
        extern "system" {
            fn ShellExecuteW(
                window: *mut std::ffi::c_void,
                operation: *const u16,
                file: *const u16,
                parameters: *const u16,
                directory: *const u16,
                show: i32,
            ) -> isize;
        }
        let wide = |value: &str| {
            std::ffi::OsStr::new(value)
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>()
        };
        let operation = wide("open");
        let target = wide(url);
        if unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                operation.as_ptr(),
                target.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            )
        } > 32
        {
            return Ok(());
        }
    }
    Err(())
}
