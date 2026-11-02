# Claude Code transport

The runner launches an unmodified, locally installed native Claude Code binary. Authentication remains in Claude Code. Nero does not read or copy Claude credentials. The login process uses the official browser flow and does not display a terminal. Nero accepts a `claude.ai` authentication status for its subscription chat.

A chat request uses an empty temporary working directory, safe and restricted modes, disabled built-in tools, an empty strict MCP configuration, and no persisted Claude Code session. Environment overrides for credentials, provider routing and custom endpoints are removed. Claude Code's managed organizational policy still applies. In particular, the official CLI documentation states that policy-configured hooks may run in safe mode; Nero does not override organizational policy.

The macOS native binary is checked with Apple code signature validation and the Claude Code signing identity. Windows validates the Authenticode signature and publisher. The official installer path is discovered relative to the user's home directory. Binary updates between validation and execution remain a filesystem race. Neither platform signature check proves the service is reachable or that an account is entitled to a particular model.

Sources:
- https://code.claude.com/docs/en/cli-reference
- https://code.claude.com/docs/en/setup#platform-code-signatures
- https://code.claude.com/docs/en/settings

The optional app-owned runtime preparation reads the official stable channel and manifest, enforces the manifest's byte count and SHA-256 digest, and requires the platform signature before making the staged binary available. A previously verified copy can be used if the update check fails. The manifest's detached GPG signature is not currently verified; the OS signature is the independent authenticity check. Runtime preparation, platform signature validation and subscription-service responses require separate platform acceptance; parser tests alone do not establish those results.
