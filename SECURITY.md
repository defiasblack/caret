# Security Policy

## Supported versions

Security fixes are provided for the latest released version of Caret and the current `main` branch when practical.

| Version | Supported |
|---|---|
| Latest release | Yes |
| `main` branch | Yes |
| Older releases | No |

## Reporting a vulnerability

Please do **not** report security vulnerabilities in public issues, discussions, pull requests, or social media posts.

Use GitHub's private vulnerability reporting feature:

1. Open the Caret repository on GitHub.
2. Select **Security**.
3. Select **Advisories**.
4. Select **Report a vulnerability**.

Include as much of the following as possible:

- A description of the vulnerability and its impact
- Affected Caret versions or commits
- Steps or proof-of-concept code that reproduce the issue
- Relevant operating system, terminal, and shell details
- Any suggested remediation

Do not include real credentials, tokens, private keys, or sensitive personal data.

## What to expect

The maintainer will aim to acknowledge a complete report within seven days. Valid reports will be investigated privately, and a fix and coordinated disclosure plan will be prepared when appropriate.

Timelines depend on severity, complexity, and maintainer availability. Please allow reasonable time for a fix before public disclosure.

## Scope

Security reports are especially helpful for issues involving:

- Arbitrary command execution
- Unsafe handling of project files or plugin manifests
- Path traversal or unintended file access
- Terminal escape-sequence injection
- Exposure of sensitive data
- Dependency vulnerabilities that are exploitable through Caret

General bugs, crashes without a security impact, and feature requests should use the normal issue templates.
