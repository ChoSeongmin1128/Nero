pub(crate) const PRIVACY_POLICY: &str = "https://choseongmin1128.github.io/Nero/privacy.html";

pub(crate) fn open_url(url: &reqwest::Url) -> Result<(), ()> {
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(());
    }
    open_target(url.as_str(), None).then_some(()).ok_or(())
}

// Hands a URL or file to the system, optionally to a specific macOS app bundle.
// Windows opens it with the registered handler and ignores the bundle.
pub(crate) fn open_target(target: &str, bundle: Option<&str>) -> bool {
    #[cfg(target_os = "macos")]
    {
        let mut command = std::process::Command::new("/usr/bin/open");
        if let Some(bundle) = bundle {
            command.args(["-b", bundle]);
        }
        command
            .arg(target)
            .output()
            .is_ok_and(|output| output.status.success())
    }
    #[cfg(target_os = "windows")]
    {
        let _ = bundle;
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
        let target = wide(target);
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                operation.as_ptr(),
                target.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            )
        };
        result > 32
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (target, bundle);
        false
    }
}
