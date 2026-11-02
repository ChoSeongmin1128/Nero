//! Signature checks must not inherit another PowerShell edition's module paths.
use std::{
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const UNAVAILABLE: &str =
    "Claude Code의 서명 검사를 실행하지 못했습니다. Nero를 업데이트하거나 다시 실행해 주세요.";

pub(super) fn validate(binary: &Path) -> Result<(), String> {
    let system = std::env::var_os("SystemRoot").ok_or(UNAVAILABLE)?;
    let powershell = PathBuf::from(system).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    // Load only the selected Windows PowerShell's built-in security module.
    // Do not interpolate file paths or weaken Authenticode/publisher validation.
    let script = r#"$ErrorActionPreference = 'Stop'; try {
Import-Module (Join-Path $PSHOME 'Modules\Microsoft.PowerShell.Security\Microsoft.PowerShell.Security.psd1') -ErrorAction Stop;
$s = Get-AuthenticodeSignature -LiteralPath $env:NERO_CLAUDE_BINARY -ErrorAction Stop;
} catch { exit 2 };
if ($s.Status -eq 'Valid' -and $null -ne $s.SignerCertificate -and $s.SignerCertificate.GetNameInfo([System.Security.Cryptography.X509Certificates.X509NameType]::SimpleName, $false) -ceq 'Anthropic, PBC') { exit 0 }; exit 1"#;
    let status = Command::new(powershell)
        .creation_flags(0x08000000)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .env_remove("PSModulePath")
        .env("NERO_CLAUDE_BINARY", binary)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| UNAVAILABLE)?;
    match status.code() {
        Some(0) => Ok(()),
        Some(1) => Err("Claude Code 실행 파일의 서명을 확인하지 못했습니다.".into()),
        _ => Err(UNAVAILABLE.into()),
    }
}
