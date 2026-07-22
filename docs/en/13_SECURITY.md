# Security & Hardening

## Security model
- What components are trusted
- Network exposure considerations

## Performance & Monitoring
- Using Prometheus metrics
- Audit logging for suspicious activity

## Audit & Compliance

When the `web` feature is enabled, lazydns automatically tracks security-relevant events via the internal event bus (visible in the WebUI Logs page and consumed by the alert engine):
- **Rate Limiting**: Monitor `rate_limit_exceeded` events to identify and block abusive clients.
- **Threat Intelligence**: Monitor `blocked_domain_query` to track attempts to reach malware or phishing domains.
- **Service Availability**: Monitor `upstream_failure` and `query_timeout` for potential network or resolver issues.
- **Validation**: Monitor `malformed_query` for potential DNS protocol attacks or misconfigured clients.

See [Audit & Query Logging](../AUDIT_USAGE_GUIDE.md) for details.

## TLS & Certificates
- DoT/DoH certificate management
- Example config for TLS

## Reporting vulnerabilities
- Contact and process for responsible disclosure

---

TODO: Add hardening checklist and recommended runtime flags.