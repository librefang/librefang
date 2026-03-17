---
name: apitester-hand-skill
version: "1.0.0"
description: "Expert knowledge for AI API testing -- HTTP reference, testing patterns, OpenAPI parsing, and load testing techniques"
runtime: prompt_only
---

# API Testing Expert Knowledge

## HTTP Reference

### Status Code Categories
| Range | Category | Common Codes |
|-------|----------|-------------|
| 2xx | Success | 200 OK, 201 Created, 204 No Content |
| 3xx | Redirection | 301 Moved, 304 Not Modified |
| 4xx | Client Error | 400 Bad Request, 401 Unauthorized, 403 Forbidden, 404 Not Found, 422 Unprocessable, 429 Too Many Requests |
| 5xx | Server Error | 500 Internal, 502 Bad Gateway, 503 Service Unavailable, 504 Gateway Timeout |

### curl Quick Reference

**GET with headers**:
```bash
curl -s -H "Authorization: Bearer TOKEN" \
  -H "Accept: application/json" \
  "https://api.example.com/endpoint"
```

**POST with JSON body**:
```bash
curl -s -X POST \
  -H "Authorization: Bearer TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"key": "value"}' \
  "https://api.example.com/endpoint"
```

**Timing information**:
```bash
curl -s -o /dev/null -w "status:%{http_code} time:%{time_total}s size:%{size_download}b" \
  "https://api.example.com/endpoint"
```

**Verbose with headers**:
```bash
curl -v -H "Authorization: Bearer TOKEN" \
  "https://api.example.com/endpoint" 2>&1
```

---

## Testing Patterns

### Functional Testing Checklist

For each endpoint, test:

1. **Happy path**: Valid request with all required parameters
2. **Missing required fields**: Omit each required field one at a time
3. **Invalid data types**: String where number expected, etc.
4. **Boundary values**: Min/max for numbers, empty strings, very long strings
5. **Special characters**: Unicode, HTML entities, SQL keywords
6. **Null values**: Explicit null vs missing field
7. **Authentication**: Valid, invalid, missing, expired tokens
8. **Authorization**: Access own resources, access others' resources
9. **Pagination**: First page, last page, beyond last page, invalid page
10. **Filtering/Sorting**: Valid filters, invalid filters, combined filters

### Test Data Patterns

```
# Safe test strings for injection testing
SQL injection:    "'; DROP TABLE users; --"
XSS:             "<script>alert('xss')</script>"
Command injection: "; cat /etc/passwd"
Path traversal:   "../../etc/passwd"
Long string:      "A" * 10000
Unicode:          "\u0000\u0001\u0002"
Email format:     "test@example.com" (use example.com domain)
```

### Response Validation

Check every response for:
```
1. Status code is expected
2. Content-Type header is correct
3. Response body parses as valid JSON/XML
4. Required fields are present
5. Field types match schema
6. No unexpected fields (strict mode)
7. No sensitive data exposure (passwords, tokens, PII)
8. Pagination metadata is correct
9. Error responses follow a consistent format
10. Response time is within acceptable range
```

---

## OpenAPI/Swagger Parsing

### Key OpenAPI 3.0 Structure

```json
{
  "openapi": "3.0.0",
  "info": {"title": "API Name", "version": "1.0"},
  "paths": {
    "/users": {
      "get": {
        "parameters": [...],
        "responses": {
          "200": {"description": "Success", "content": {"application/json": {"schema": {...}}}}
        }
      },
      "post": {
        "requestBody": {"content": {"application/json": {"schema": {...}}}},
        "responses": {...}
      }
    }
  },
  "components": {
    "schemas": {...},
    "securitySchemes": {...}
  }
}
```

### Extracting Test Cases from OpenAPI

For each path + method combination:
1. Extract required parameters (path, query, header)
2. Extract request body schema (for POST/PUT/PATCH)
3. Extract expected response schemas per status code
4. Note security requirements
5. Generate positive and negative test cases

---

## Load Testing Techniques

### Ramp-Up Pattern
```
Phase 1: 10 concurrent users for 30 seconds (warm up)
Phase 2: 50 concurrent users for 60 seconds (moderate load)
Phase 3: 100 concurrent users for 60 seconds (high load)
Phase 4: 200 concurrent users for 30 seconds (stress test)
Phase 5: 10 concurrent users for 30 seconds (recovery check)
```

### Key Metrics to Track
| Metric | Formula | Acceptable | Warning | Critical |
|--------|---------|-----------|---------|----------|
| Avg Response Time | sum(times)/count | <200ms | 200-500ms | >500ms |
| P95 Response Time | 95th percentile | <500ms | 500ms-1s | >1s |
| Error Rate | errors/total*100 | <1% | 1-5% | >5% |
| Throughput | requests/second | Depends | Decreasing | Dropping |

### Shell-Based Load Testing

Simple concurrent requests:
```bash
# Send 50 concurrent requests
for i in $(seq 1 50); do
  curl -s -o /dev/null -w "%{http_code} %{time_total}\n" \
    -H "Authorization: Bearer TOKEN" \
    "https://api.example.com/endpoint" &
done
wait
```

Sustained load test with timing:
```bash
# 100 requests, 10 at a time
for batch in $(seq 1 10); do
  for i in $(seq 1 10); do
    curl -s -o /dev/null -w "%{http_code} %{time_total}\n" \
      "https://api.example.com/endpoint" &
  done
  wait
  sleep 1
done
```

---

## Security Testing Reference

### OWASP API Security Top 10

1. **Broken Object Level Authorization**: Access other users' data by changing IDs
2. **Broken Authentication**: Weak auth mechanisms, missing rate limits
3. **Broken Object Property Level Authorization**: Mass assignment, excessive data exposure
4. **Unrestricted Resource Consumption**: Missing rate limits, large payloads
5. **Broken Function Level Authorization**: Access admin endpoints as regular user
6. **Unrestricted Access to Sensitive Business Flows**: Abuse of purchase, reservation, etc.
7. **Server-Side Request Forgery**: API fetches attacker-controlled URLs
8. **Security Misconfiguration**: Default configs, verbose errors, missing headers
9. **Improper Inventory Management**: Exposed old API versions, debug endpoints
10. **Unsafe Consumption of APIs**: Trusting third-party API responses without validation

### Security Headers to Check

```
Strict-Transport-Security: max-age=31536000
X-Content-Type-Options: nosniff
X-Frame-Options: DENY
Content-Security-Policy: default-src 'self'
X-XSS-Protection: 1; mode=block
Cache-Control: no-store (for sensitive endpoints)
```

---

## Test Report Templates

### Per-Endpoint Result Format
```json
{
  "endpoint": "/api/users",
  "method": "GET",
  "tests": [
    {"name": "Happy path", "status": "PASS", "code": 200, "time_ms": 45},
    {"name": "Missing auth", "status": "PASS", "code": 401, "time_ms": 12},
    {"name": "Invalid ID", "status": "FAIL", "code": 500, "time_ms": 230, "note": "Expected 404, got 500"}
  ]
}
```

### Regression Detection
Compare two test runs:
```
Field Changed:  response.data[].email field removed
Impact:         Breaking change for API consumers
Severity:       HIGH
First Seen:     2025-01-15 run
Previous Value: string (email format)
Current Value:  field absent
```
