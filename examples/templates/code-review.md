# Code Review

> Automated code review — checks architecture, security, performance, and code quality.

## Tasks

### T1: Architecture & Structure Overview
Read the project structure, entry points, dependencies, and key modules. Write a summary of the architecture to REVIEW.md including: tech stack, module organization, data flow, and external dependencies.

### T2: Error Handling Audit
Scan for error handling patterns across the codebase. Flag: bare unwrap()/expect(), swallowed errors (empty catch blocks), missing error types, inconsistent error propagation, panics in library code. Document findings in REVIEW.md.

### T3: Security Review
Check for common vulnerabilities: hardcoded secrets/API keys, SQL injection, path traversal, unsafe deserialization, missing input validation, CORS misconfiguration, improper auth checks, use of eval/exec. Document findings with file:line references in REVIEW.md.

### T4: Dead Code & Unused Dependencies
Identify unused imports, dead functions, unreachable code paths, and unnecessary dependencies. List candidates for removal in REVIEW.md.

### T5: Test Coverage Analysis
Identify critical paths that lack test coverage. List specific functions/modules that should have tests but don't. Suggest concrete test cases. Document in REVIEW.md.

### T6: Performance Red Flags
Look for: N+1 queries, unbounded allocations, blocking calls in async code, missing database indexes, O(n²) algorithms where O(n) is possible, large synchronous file reads, memory leaks. Document in REVIEW.md.

### T7: Code Quality & Style
Check for: inconsistent naming conventions, overly complex functions (>50 lines), deep nesting (>3 levels), magic numbers, missing documentation on public APIs, copy-pasted code blocks. Document in REVIEW.md.

### T8: Compile Review Report
Consolidate all findings into a final REVIEW.md with sections for each category. Rate each finding as 🔴 Critical, 🟡 Warning, or 🔵 Info. Include specific file:line references and suggested fixes.
