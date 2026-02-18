# Security Audit

> Deep security review — authentication, injection, secrets, dependencies, and OWASP top 10.

## Tasks

### T1: Secrets & Credentials Scan
Search the entire codebase and git history for hardcoded secrets: API keys, passwords, tokens, private keys, connection strings. Check .env files are in .gitignore. Flag any secrets committed to version control. Write findings to SECURITY-AUDIT.md.

### T2: Authentication & Authorization Review
Review all auth flows: login, registration, password reset, session management, token handling. Check for: missing auth on endpoints, broken access control, privilege escalation paths, insecure token storage, missing CSRF protection. Document in SECURITY-AUDIT.md.

### T3: Injection Vulnerability Scan
Check for: SQL injection (raw queries, string concatenation), XSS (unescaped user input in HTML), command injection (shell exec with user input), path traversal (user-controlled file paths), LDAP/NoSQL injection. Document with file:line refs.

### T4: Dependency Vulnerability Check
Run the project's dependency audit tool (npm audit, cargo audit, pip-audit, etc.). List all known vulnerabilities with severity ratings. Identify outdated dependencies with known CVEs. Document in SECURITY-AUDIT.md.

### T5: Input Validation & Data Sanitization
Review all user input entry points (API params, form fields, file uploads, URL params). Check for: missing validation, type coercion issues, buffer overflow potential, unsafe deserialization, unrestricted file uploads. Document findings.

### T6: Compile Security Report
Consolidate all findings into SECURITY-AUDIT.md with: executive summary, findings by severity (🔴 Critical / 🟡 High / 🟠 Medium / 🔵 Low), remediation steps for each finding, and a prioritized action plan.
