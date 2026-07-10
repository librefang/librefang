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

---

## Worked Examples

### Example 1: Testing a REST API CRUD Endpoint

Full test suite for a `/api/users` resource covering create, read, update, delete, and edge cases.

**Setup — Create a test user**:
```bash
# POST /api/users — create
RESPONSE=$(curl -s -w "\n%{http_code}" -X POST \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"name": "Ada Lovelace", "email": "ada@example.com", "role": "engineer"}' \
  "https://api.example.com/api/users")

BODY=$(echo "$RESPONSE" | sed '$d')
STATUS=$(echo "$RESPONSE" | tail -1)

# Expect 201 Created
[ "$STATUS" = "201" ] && echo "PASS: Create user" || echo "FAIL: Expected 201, got $STATUS"

# Extract ID for subsequent tests
USER_ID=$(echo "$BODY" | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
```

**Read operations**:
```bash
# GET /api/users — list all
curl -s -H "Authorization: Bearer $TOKEN" \
  "https://api.example.com/api/users" | python3 -m json.tool

# GET /api/users/:id — single user
curl -s -H "Authorization: Bearer $TOKEN" \
  "https://api.example.com/api/users/$USER_ID" | python3 -m json.tool

# GET /api/users/nonexistent-id — expect 404
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -H "Authorization: Bearer $TOKEN" \
  "https://api.example.com/api/users/00000000-0000-0000-0000-000000000000")
[ "$STATUS" = "404" ] && echo "PASS: 404 for missing user" || echo "FAIL: Expected 404, got $STATUS"
```

**Update operations**:
```bash
# PUT /api/users/:id — full update
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -X PUT \
  -H "Content-Type: application/json" -H "Authorization: Bearer $TOKEN" \
  -d '{"name": "Ada Lovelace", "email": "ada.updated@example.com", "role": "lead"}' \
  "https://api.example.com/api/users/$USER_ID")
[ "$STATUS" = "200" ] && echo "PASS: Full update" || echo "FAIL: Expected 200, got $STATUS"

# PATCH — partial update (expect 200); also test invalid data (expect 400/422)
```

**Delete and verify**:
```bash
# DELETE /api/users/:id
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -X DELETE \
  -H "Authorization: Bearer $TOKEN" \
  "https://api.example.com/api/users/$USER_ID")
[ "$STATUS" = "204" ] || [ "$STATUS" = "200" ] && echo "PASS: Delete user" || echo "FAIL: Expected 2xx, got $STATUS"

# GET deleted user — expect 404 or 410
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -H "Authorization: Bearer $TOKEN" \
  "https://api.example.com/api/users/$USER_ID")
[ "$STATUS" = "404" ] || [ "$STATUS" = "410" ] && echo "PASS: Deleted user gone" || echo "FAIL: Expected 404/410, got $STATUS"

# DELETE again — idempotency check
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -X DELETE \
  -H "Authorization: Bearer $TOKEN" \
  "https://api.example.com/api/users/$USER_ID")
[ "$STATUS" = "404" ] || [ "$STATUS" = "204" ] && echo "PASS: Idempotent delete" || echo "FAIL: Got $STATUS"
```

**Edge cases to test**: duplicate create (expect 409), empty body (expect 400/422), extra unknown fields (verify ignored or rejected, not persisted).

### Example 2: Testing an Authenticated API with Rate Limiting

Scenario: API uses Bearer tokens, tokens expire after 1 hour, rate limit is 100 requests/minute.

**Token lifecycle testing**:
```bash
# Step 1: Obtain token
AUTH_RESPONSE=$(curl -s -X POST \
  -H "Content-Type: application/json" \
  -d '{"client_id": "myapp", "client_secret": "secret", "grant_type": "client_credentials"}' \
  "https://api.example.com/oauth/token")

ACCESS_TOKEN=$(echo "$AUTH_RESPONSE" | python3 -c "import sys,json; print(json.load(sys.stdin)['access_token'])")
EXPIRES_IN=$(echo "$AUTH_RESPONSE" | python3 -c "import sys,json; print(json.load(sys.stdin)['expires_in'])")
echo "Token obtained, expires in ${EXPIRES_IN}s"

# Step 2: Use token — expect 200
STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  "https://api.example.com/api/protected")
[ "$STATUS" = "200" ] && echo "PASS: Valid token accepted" || echo "FAIL: Got $STATUS"

# Step 3: Use expired/invalid token — expect 401
STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
  -H "Authorization: Bearer expired.token.here" \
  "https://api.example.com/api/protected")
[ "$STATUS" = "401" ] && echo "PASS: Expired token rejected" || echo "FAIL: Got $STATUS"

# Step 4: Missing Authorization header — expect 401
STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
  "https://api.example.com/api/protected")
[ "$STATUS" = "401" ] && echo "PASS: No auth rejected" || echo "FAIL: Got $STATUS"

# Step 5: Malformed header — expect 401
STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
  -H "Authorization: NotBearer $ACCESS_TOKEN" \
  "https://api.example.com/api/protected")
[ "$STATUS" = "401" ] && echo "PASS: Bad scheme rejected" || echo "FAIL: Got $STATUS"
```

**Rate limit testing**:
```bash
# Hit the endpoint rapidly and watch for 429
RESULTS_FILE=$(mktemp)
for i in $(seq 1 120); do
  curl -s -o /dev/null -w "%{http_code}\n" \
    -H "Authorization: Bearer $ACCESS_TOKEN" \
    "https://api.example.com/api/data" >> "$RESULTS_FILE" &
done
wait

# Count status codes
echo "=== Rate Limit Results ==="
sort "$RESULTS_FILE" | uniq -c | sort -rn
# Expected: ~100 x 200, ~20 x 429

# Check rate limit headers on a single request
curl -s -D- -o /dev/null \
  -H "Authorization: Bearer $ACCESS_TOKEN" \
  "https://api.example.com/api/data" | grep -i "x-ratelimit"
# Expected headers:
#   X-RateLimit-Limit: 100
#   X-RateLimit-Remaining: 99
#   X-RateLimit-Reset: 1700000060

rm "$RESULTS_FILE"
```

**Backoff strategy**: On 429, respect `Retry-After` header. Use exponential backoff (1s, 2s, 4s...) as fallback. Verify the API returns `X-RateLimit-Reset` for client scheduling.

### Example 3: Testing a Webhook Endpoint

Scenario: Your API accepts webhook callbacks at `POST /webhooks/payment` with HMAC-SHA256 signature verification.

**Payload and signature generation**:
```bash
WEBHOOK_SECRET="whsec_test_secret_key_12345"
PAYLOAD='{"event":"payment.completed","data":{"id":"pay_123","amount":4999,"currency":"usd"}}'
TIMESTAMP=$(date +%s)
SIGNATURE=$(printf "%s.%s" "$TIMESTAMP" "$PAYLOAD" | openssl dgst -sha256 -hmac "$WEBHOOK_SECRET" | awk '{print $2}')

# Valid webhook delivery
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  -H "Content-Type: application/json" \
  -H "X-Webhook-Signature: t=$TIMESTAMP,v1=$SIGNATURE" \
  -H "X-Webhook-Id: wh_evt_001" \
  -d "$PAYLOAD" \
  "https://api.example.com/webhooks/payment")
[ "$STATUS" = "200" ] || [ "$STATUS" = "204" ] && echo "PASS: Valid webhook accepted" || echo "FAIL: Got $STATUS"
```

**Signature verification tests**:
```bash
# Wrong signature — expect 401 or 403
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  -H "Content-Type: application/json" \
  -H "X-Webhook-Signature: t=$TIMESTAMP,v1=badsignaturevalue" \
  -d "$PAYLOAD" \
  "https://api.example.com/webhooks/payment")
[ "$STATUS" = "401" ] || [ "$STATUS" = "403" ] && echo "PASS: Bad signature rejected" || echo "FAIL: Got $STATUS"

# Missing signature header — expect 401
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  -H "Content-Type: application/json" \
  -d "$PAYLOAD" \
  "https://api.example.com/webhooks/payment")
[ "$STATUS" = "401" ] && echo "PASS: Missing signature rejected" || echo "FAIL: Got $STATUS"

# Stale timestamp (replay attack) — expect 403
OLD_TIMESTAMP=$((TIMESTAMP - 600))
OLD_SIGNATURE=$(printf "%s.%s" "$OLD_TIMESTAMP" "$PAYLOAD" | openssl dgst -sha256 -hmac "$WEBHOOK_SECRET" | awk '{print $2}')
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  -H "Content-Type: application/json" \
  -H "X-Webhook-Signature: t=$OLD_TIMESTAMP,v1=$OLD_SIGNATURE" \
  -d "$PAYLOAD" \
  "https://api.example.com/webhooks/payment")
[ "$STATUS" = "403" ] && echo "PASS: Stale timestamp rejected" || echo "FAIL: Got $STATUS"
```

**Also test**: idempotency (same `X-Webhook-Id` sent twice — should be processed once), invalid/empty payloads (expect 400).

---

## Authentication Testing Patterns

### OAuth 2.0 Flow Testing

**Authorization Code flow**:
```bash
# Step 1: Initiate authorization — verify redirect
AUTHORIZE_URL="https://api.example.com/oauth/authorize?response_type=code&client_id=myapp&redirect_uri=https://myapp.example.com/callback&scope=read+write&state=random_state_123"
STATUS=$(curl -s -o /dev/null -w "%{http_code}" "$AUTHORIZE_URL")
[ "$STATUS" = "302" ] || [ "$STATUS" = "200" ] && echo "PASS: Auth endpoint reachable" || echo "FAIL: Got $STATUS"

# Step 2: Exchange authorization code for token
TOKEN_RESPONSE=$(curl -s -X POST \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=authorization_code&code=AUTH_CODE_HERE&redirect_uri=https://myapp.example.com/callback&client_id=myapp&client_secret=secret" \
  "https://api.example.com/oauth/token")
echo "$TOKEN_RESPONSE" | python3 -m json.tool
# Verify: access_token, refresh_token, expires_in, token_type present

# Step 3: Use invalid authorization code — expect 400
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=authorization_code&code=INVALID_CODE&redirect_uri=https://myapp.example.com/callback&client_id=myapp&client_secret=secret" \
  "https://api.example.com/oauth/token")
[ "$STATUS" = "400" ] && echo "PASS: Invalid code rejected" || echo "FAIL: Got $STATUS"

# Step 4: Reuse authorization code — must fail (codes are single-use)
# Use the same AUTH_CODE_HERE again
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=authorization_code&code=AUTH_CODE_HERE&redirect_uri=https://myapp.example.com/callback&client_id=myapp&client_secret=secret" \
  "https://api.example.com/oauth/token")
[ "$STATUS" = "400" ] && echo "PASS: Code reuse rejected" || echo "FAIL: Got $STATUS"
```

**Client Credentials flow**: Same pattern as above with `grant_type=client_credentials`. Test: valid credentials (expect `access_token`), invalid secret (expect 401), invalid `grant_type` (expect 400).

**Refresh Token flow**: Exchange `grant_type=refresh_token` with `refresh_token=$REFRESH_TOKEN`. Verify: new `access_token` returned, old refresh token invalidated if rotation is enabled (reuse should return 400/401).

### JWT Validation Testing

Test each type of JWT failure independently:

| Test Case | Token Modification | Expected Status | Expected Error |
|-----------|-------------------|-----------------|----------------|
| Expired token | Set `exp` to past timestamp | 401 | `token_expired` |
| Not-yet-valid | Set `nbf` to future timestamp | 401 | `token_not_yet_valid` |
| Wrong signature | Sign with different key | 401 | `invalid_signature` |
| Malformed token | Remove a segment | 401 | `malformed_token` |
| Missing `sub` claim | Remove `sub` from payload | 401 | `missing_claims` |
| Wrong audience | Set `aud` to different app | 401 | `invalid_audience` |
| Wrong issuer | Set `iss` to unknown issuer | 401 | `invalid_issuer` |
| Algorithm none attack | Set `alg: none`, remove signature | 401 | `invalid_algorithm` |

```bash
# Generate a test JWT with wrong signature (using python3 as a helper)
HEADER=$(echo -n '{"alg":"HS256","typ":"JWT"}' | base64 | tr -d '=' | tr '+/' '-_')
PAYLOAD=$(echo -n '{"sub":"user123","exp":9999999999}' | base64 | tr -d '=' | tr '+/' '-_')
BAD_SIG=$(echo -n "fakesignature" | base64 | tr -d '=' | tr '+/' '-_')
BAD_JWT="${HEADER}.${PAYLOAD}.${BAD_SIG}"

STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
  -H "Authorization: Bearer $BAD_JWT" \
  "https://api.example.com/api/protected")
[ "$STATUS" = "401" ] && echo "PASS: Bad JWT signature rejected" || echo "FAIL: Got $STATUS"

# Algorithm "none" attack
NONE_HEADER=$(echo -n '{"alg":"none","typ":"JWT"}' | base64 | tr -d '=' | tr '+/' '-_')
NONE_JWT="${NONE_HEADER}.${PAYLOAD}."
STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
  -H "Authorization: Bearer $NONE_JWT" \
  "https://api.example.com/api/protected")
[ "$STATUS" = "401" ] && echo "PASS: alg:none attack blocked" || echo "FAIL: Got $STATUS — SECURITY RISK"
```

### API Key Testing Patterns

```bash
# Valid API key in header
STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
  -H "X-API-Key: valid_key_abc123" \
  "https://api.example.com/api/data")
[ "$STATUS" = "200" ] && echo "PASS: Valid API key" || echo "FAIL: Got $STATUS"
```

**Also test**: key in query param (if supported), revoked key (expect 401/403), empty key (expect 401), read-only key attempting write (expect 403).

### Session-Based Auth Testing

Test pattern: login (capture `Set-Cookie`), use cookie for authenticated request (expect 200), logout, reuse cookie (expect 401). Also verify session fixation prevention — session ID should rotate on login.

---

## Contract Testing

### Schema Validation Techniques

Validate API responses against a JSON Schema using `python3 -c "from jsonschema import validate; ..."`:

```bash
# Fetch response and validate against schema file
curl -s -H "Authorization: Bearer $TOKEN" \
  "https://api.example.com/api/users/user_001" | python3 -c "
import sys, json
from jsonschema import validate, ValidationError
schema = json.load(open('/tmp/user_schema.json'))
try:
    validate(instance=json.load(sys.stdin), schema=schema)
    print('PASS: Schema valid')
except ValidationError as e:
    print(f'FAIL: {e.message}')
"
```

Schema should define `required` fields, property `type`/`format`/`enum` constraints, and `additionalProperties: false` for strict mode.

### Breaking Change Detection

Compare current response structure against a recorded baseline:

```bash
# Helper: extract JSON shape as "path: type" lines
extract_shape() {
  curl -s -H "Authorization: Bearer $TOKEN" "$1" | python3 -c "
import sys, json
def shape(obj, prefix=''):
    s = {}
    if isinstance(obj, dict):
        for k, v in obj.items():
            p = f'{prefix}.{k}' if prefix else k
            s[p] = type(v).__name__; s.update(shape(v, p))
    elif isinstance(obj, list) and obj:
        s[f'{prefix}[]'] = type(obj[0]).__name__; s.update(shape(obj[0], f'{prefix}[]'))
    return s
for p, t in sorted(shape(json.load(sys.stdin)).items()): print(f'{p}: {t}')
"
}

# Record baseline once, then diff against current
extract_shape "https://api.example.com/api/users/user_001" > /tmp/api_baseline.txt
# ... later ...
extract_shape "https://api.example.com/api/users/user_001" > /tmp/api_current.txt
diff /tmp/api_baseline.txt /tmp/api_current.txt && echo "PASS: No schema changes" || echo "WARN: Schema changed"
```

### Backward Compatibility Checklist

When a new API version is deployed, verify that existing consumers are not broken:

| Check | How to Test | Severity |
|-------|------------|----------|
| Removed fields | Diff response shape against baseline | **HIGH** — breaks consumers |
| Renamed fields | Diff response keys | **HIGH** — breaks consumers |
| Changed field type | Compare type of each field | **HIGH** — breaks deserialization |
| New required request field | Send old-format request | **HIGH** — breaks callers |
| Changed enum values | Check if old values still accepted | **MEDIUM** — breaks validation |
| Changed error format | Compare error response structure | **MEDIUM** — breaks error handlers |
| Changed status codes | Compare response codes for same input | **MEDIUM** — breaks status checks |
| New optional fields | Verify response still parses | **LOW** — usually safe |
| Pagination format change | Test with existing page params | **MEDIUM** — breaks pagination loops |

### Consumer-Driven Contract Testing

Concept: Each API consumer defines the minimum contract they need (required fields, forbidden fields, expected status codes). The provider runs all consumer contracts in CI.

```json
{
  "consumer": "mobile-app-v2",
  "provider": "user-service",
  "interactions": [
    {
      "description": "get user profile",
      "request": {"method": "GET", "path": "/api/users/me", "headers": {"Authorization": "Bearer valid_token"}},
      "response": {"status": 200, "body_contains": ["id", "name", "email"], "body_must_not_contain": ["password", "internal_id"]}
    }
  ]
}
```

Runner approach: iterate interactions, execute each request with curl, verify status code matches and required/forbidden fields are present/absent in the response body.

---

## Performance Testing Deep Dive

### Load Test Types

| Type | Purpose | Pattern |
|------|---------|---------|
| **Soak** | Detect memory leaks, connection pool exhaustion | Steady traffic (e.g., 5 req/s) for hours; compare first-quarter vs last-quarter response times |
| **Spike** | Verify graceful handling of sudden bursts | Baseline → 10x-20x burst → recovery; check error rate and recovery time |
| **Stress** | Find the breaking point | Incrementally increase concurrency until errors begin |

### Stress Testing (Representative Example)

Incrementally increase load until errors begin — adapt the same pattern for soak (fixed concurrency, long duration) or spike (sudden burst) testing:

```bash
echo "concurrency,success_rate,avg_time,p95_time" > /tmp/stress_results.csv
for CONCURRENCY in 10 25 50 100 200 500; do
  RESULTS=$(mktemp)
  for i in $(seq 1 $CONCURRENCY); do
    curl -s -o /dev/null -w "%{http_code} %{time_total}\n" \
      -H "Authorization: Bearer $TOKEN" \
      "https://api.example.com/api/data" >> "$RESULTS" &
  done
  wait

  TOTAL=$(wc -l < "$RESULTS")
  SUCCESS=$(grep -c "^200" "$RESULTS")
  AVG_TIME=$(awk '{sum+=$2; n++} END {printf "%.3f", sum/n}' "$RESULTS")
  P95_TIME=$(awk '{print $2}' "$RESULTS" | sort -n | awk -v p=0.95 'NR==1{n=0} {a[n++]=$1} END {print a[int(n*p)]}')

  echo "$CONCURRENCY,$((SUCCESS*100/TOTAL))%,$AVG_TIME,$P95_TIME" >> /tmp/stress_results.csv
  echo "Concurrency $CONCURRENCY: ${SUCCESS}/${TOTAL} success, avg=${AVG_TIME}s, p95=${P95_TIME}s"

  rm "$RESULTS"
  sleep 3  # Let the server recover between steps
done

echo "=== Stress Test Summary ==="
column -t -s',' /tmp/stress_results.csv
```

### Latency Percentile Analysis

Collect many response times (e.g., 1000 with concurrency capped at 20), then compute p50/p75/p90/p95/p99 percentiles. Compare first-quarter vs last-quarter averages to detect degradation over time.

```bash
# Collect response times
TIMES_FILE=$(mktemp)
for i in $(seq 1 1000); do
  curl -s -o /dev/null -w "%{time_total}\n" \
    -H "Authorization: Bearer $TOKEN" \
    "https://api.example.com/api/data" >> "$TIMES_FILE" &
  [ $((i % 20)) -eq 0 ] && wait
done
wait
# Sort and compute percentiles with: sort -n "$TIMES_FILE" | python3 ...
rm "$TIMES_FILE"
```

### Connection Pool Testing

- **Keep-alive reuse**: Send multiple URLs in one curl call with `Connection: keep-alive`; second/third requests should show near-zero `time_connect`.
- **Connection exhaustion**: Open 500 concurrent keep-alive connections; watch for 503 or connection refused errors.

---

## Common API Bugs & How to Find Them

### N+1 Query Detection

Response time should not scale linearly with data size. If fetching 10 items takes 100ms but 100 items takes 1000ms, the API likely has an N+1 query problem.

```bash
# Compare response times for different page sizes
for SIZE in 1 10 50 100; do
  TIME=$(curl -s -o /dev/null -w "%{time_total}" \
    -H "Authorization: Bearer $TOKEN" \
    "https://api.example.com/api/orders?per_page=$SIZE")
  echo "page_size=$SIZE  time=${TIME}s"
done
# Expected (healthy): Times should NOT scale linearly
#   page_size=1    time=0.045s
#   page_size=10   time=0.052s
#   page_size=50   time=0.078s
#   page_size=100  time=0.110s
# Red flag (N+1): Times scale roughly linearly
#   page_size=1    time=0.045s
#   page_size=10   time=0.350s
#   page_size=50   time=1.600s
#   page_size=100  time=3.200s
```

### Race Condition Testing

```bash
# Concurrent counter increment — final value should equal attempt count
curl -s -X PUT -H "Content-Type: application/json" -H "Authorization: Bearer $TOKEN" \
  -d '{"value": 0}' "https://api.example.com/api/counters/counter_001"

for i in $(seq 1 50); do
  curl -s -X POST -H "Content-Type: application/json" -H "Authorization: Bearer $TOKEN" \
    -d '{"increment": 1}' "https://api.example.com/api/counters/counter_001/increment" &
done
wait

FINAL=$(curl -s -H "Authorization: Bearer $TOKEN" \
  "https://api.example.com/api/counters/counter_001" | python3 -c "import sys,json; print(json.load(sys.stdin)['value'])")
[ "$FINAL" = "50" ] && echo "PASS: No race condition" || echo "FAIL: Lost $((50 - FINAL)) increments"
```

**Optimistic locking test**: Two concurrent PUTs with same `If-Match` ETag — one should get 200, the other 409 Conflict.

### Pagination Edge Cases

| Input | Expected Behavior |
|-------|------------------|
| `page=0` | 400, or treat as page 1 |
| `page=-1` | 400 |
| `page=99999` (beyond data) | 200 with empty array, not error |
| `per_page=0` | 400 or use default |
| `per_page=100000` | Capped to server max (e.g., 100) |
| Delete item mid-pagination | No items skipped or duplicated on next page |

### Timezone Handling Bugs

Test that equivalent timestamps in different offset formats are stored identically:

```bash
# All four represent the same moment — stored values should be equivalent
for TZ in "2025-06-15T10:00:00Z" "2025-06-15T10:00:00+00:00" "2025-06-15T18:00:00+08:00" "2025-06-15T05:00:00-05:00"; do
  STORED=$(curl -s -X POST -H "Content-Type: application/json" -H "Authorization: Bearer $TOKEN" \
    -d "{\"title\": \"tz_test\", \"scheduled_at\": \"$TZ\"}" \
    "https://api.example.com/api/events" | python3 -c "import sys,json; print(json.load(sys.stdin).get('scheduled_at','ERROR'))")
  echo "Input: $TZ -> Stored: $STORED"
done
```

**Also test**: date range filters across timezone boundaries, midnight boundary inclusion/exclusion behavior.

### Character Encoding Issues

Test that the API correctly round-trips various Unicode inputs. Key test values:

| Category | Example | What Breaks |
|----------|---------|-------------|
| Emoji | `Hello 🌍🚀` | UTF-8 4-byte sequences, database column width |
| CJK | `你好世界` | Multi-byte encoding, string length vs byte length |
| Diacritics | `café` (composed vs decomposed) | Unicode normalization (NFC vs NFD) |
| Zero-width | `test\u200Bword` | Invisible characters in search/comparison |
| Null byte | `test\u0000value` | String termination in C-based systems |

```bash
# Round-trip test pattern: POST a value, verify GET returns the same
for VALUE in "Hello 🌍🚀" "你好世界" "café"; do
  RESPONSE=$(curl -s -X POST -H "Content-Type: application/json; charset=utf-8" \
    -H "Authorization: Bearer $TOKEN" \
    -d "{\"name\": \"$VALUE\"}" \
    "https://api.example.com/api/items")
  RETURNED=$(echo "$RESPONSE" | python3 -c "import sys,json; print(json.load(sys.stdin).get('name','ERROR'))")
  [ "$VALUE" = "$RETURNED" ] && echo "PASS: $VALUE" || echo "FAIL: sent='$VALUE' got='$RETURNED'"
done
```

---

## Advanced curl Patterns

### File Upload Testing

```bash
# Single file upload
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  -H "Authorization: Bearer $TOKEN" \
  -F "file=@/path/to/document.pdf" \
  -F "description=Test upload" \
  "https://api.example.com/api/uploads")
echo "Single file upload: $STATUS"

# Multiple file upload
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -X POST \
  -H "Authorization: Bearer $TOKEN" \
  -F "files[]=@/path/to/file1.png" \
  -F "files[]=@/path/to/file2.png" \
  -F "category=images" \
  "https://api.example.com/api/uploads/batch")
echo "Multi-file upload: $STATUS"
```

**Edge cases to also test**: oversized files (expect 413), wrong content type (e.g., `script.sh` declared as `image/png`), zero-byte files (expect 400).

### Multipart Form Data

```bash
# Mixed multipart: file + JSON metadata
curl -s -X POST \
  -H "Authorization: Bearer $TOKEN" \
  -F "metadata={\"title\":\"Report Q4\",\"tags\":[\"finance\",\"quarterly\"]};type=application/json" \
  -F "file=@/path/to/report.pdf" \
  "https://api.example.com/api/documents"

# Form-encoded data (not JSON)
curl -s -X POST \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "username=testuser&password=testpass&remember=true" \
  "https://api.example.com/auth/login"
```

### Cookie-Based Session Testing

```bash
# Full session lifecycle with cookie jar
COOKIE_JAR=$(mktemp)

# Login — store cookies
curl -s -c "$COOKIE_JAR" -X POST \
  -H "Content-Type: application/json" \
  -d '{"username": "testuser", "password": "testpass"}' \
  "https://api.example.com/auth/login"

# Authenticated request — send cookies
curl -s -b "$COOKIE_JAR" -c "$COOKIE_JAR" \
  "https://api.example.com/api/profile"

# Logout and verify session invalidated
curl -s -b "$COOKIE_JAR" -c "$COOKIE_JAR" -X POST \
  "https://api.example.com/auth/logout"
STATUS=$(curl -s -b "$COOKIE_JAR" -o /dev/null -w "%{http_code}" \
  "https://api.example.com/api/profile")
[ "$STATUS" = "401" ] && echo "PASS: Session invalidated" || echo "FAIL: Got $STATUS"
rm "$COOKIE_JAR"
```

**Also verify**: HttpOnly/Secure/SameSite cookie attributes, session ID rotation on login (session fixation prevention).

### Following Redirects

```bash
# Follow redirects automatically
curl -s -L -o /dev/null -w "final_url:%{url_effective} status:%{http_code} redirects:%{num_redirects}\n" \
  "https://api.example.com/old-endpoint"

# Don't follow — inspect redirect target
curl -s -D- -o /dev/null \
  "https://api.example.com/old-endpoint" | grep -i "location:"

# Open redirect vulnerability test
LOCATION=$(curl -s -D- -o /dev/null \
  "https://api.example.com/redirect?url=https://evil.example.com" | grep -i "location:" | tr -d '\r')
echo "$LOCATION" | grep -q "evil.example.com" && echo "FAIL: Open redirect vulnerability" || echo "PASS: Redirect restricted"

# HTTP to HTTPS redirect check
STATUS=$(curl -s -o /dev/null -w "%{http_code}" "http://api.example.com/api/data")
[ "$STATUS" = "301" ] || [ "$STATUS" = "308" ] && echo "PASS: HTTP redirects to HTTPS" || echo "WARN: No HTTPS redirect (got $STATUS)"
```

### HEAD, OPTIONS, and CORS

```bash
# HEAD request — verify no body returned
curl -s -I -w "status:%{http_code} size:%{size_download}\n" \
  -H "Authorization: Bearer $TOKEN" \
  "https://api.example.com/api/data"

# OPTIONS request — check CORS and allowed methods
curl -s -X OPTIONS -D- -o /dev/null \
  -H "Origin: https://myapp.example.com" \
  -H "Access-Control-Request-Method: POST" \
  "https://api.example.com/api/data" | grep -iE "(allow|access-control)"
```

---

## Chaos & Fault Injection Patterns

| Fault | How to Inject | Expected Behavior |
|-------|--------------|-------------------|
| Slow client | `curl --limit-rate 1k` | Server does not hold connection indefinitely; times out gracefully |
| Partial body | Pipe truncated JSON via `echo '{"name":' \| curl -d @-` | 400 Bad Request, not 500 |
| Huge header | `-H "X-Pad: $(python3 -c 'print("A"*16000)')"` | 431 Request Header Fields Too Large or 400 |
| Concurrent duplicate | Fire same POST with idempotency key 50x in parallel | Exactly one resource created; others get 409 or identical response |
| Connection reset | `curl --max-time 0.001` (client aborts mid-response) | Server logs show no crash; subsequent requests succeed |
| Malformed encoding | Send `Content-Type: application/json; charset=iso-8859-1` with UTF-8 body | API rejects or correctly transcodes; no mojibake in stored data |

---

## API Versioning Test Strategies

When an API exposes multiple versions, verify isolation and deprecation handling:

| Test | Method | Expected |
|------|--------|----------|
| Old version still works | `GET /api/v1/resource` | 200 with v1 schema (or 410 if sunset) |
| New version returns new schema | `GET /api/v2/resource` | 200 with v2 fields present |
| Version via header | `Accept: application/vnd.api.v2+json` | Response matches v2 schema |
| Unsupported version | `GET /api/v99/resource` | 404 or 400, not fallback to latest |
| Sunset header | Check `Sunset:` and `Deprecation:` headers on old versions | Headers present with valid dates |
| Cross-version mutation | Create in v1, read in v2 and vice versa | Data accessible in both; fields map correctly |

---

## GraphQL-Specific Testing Patterns

When the target exposes a GraphQL endpoint (`POST /graphql`):

- **Introspection**: Send `{ __schema { types { name } } }` — should be disabled in production (expect error), or return schema if intentionally public
- **Query depth attack**: Nest a query 15+ levels deep (e.g. `{ user { friends { friends { ... } } } }`) — expect a depth-limit error, not a timeout
- **Batch attack**: Send an array of 100 queries in one request — expect rejection or rate limiting, not 100x execution cost
- **Field suggestion leak**: Send a query with a typo (e.g. `{ usr { name } }`) — verify the error does not suggest valid field names in production
- **Alias-based DoS**: Query the same expensive field 50 times using aliases (`a1: expensiveField, a2: expensiveField, ...`) — expect query complexity rejection
- **Mutation authorization**: Execute mutations for other users' resources — expect authorization errors identical to REST BOLA checks
- **N+1 detection**: Query a list with nested relations (`{ users { orders { items } } }`) — linear response time scaling signals N+1

---

## Webhook Reliability Testing Patterns

Beyond signature verification (covered in worked examples), test delivery reliability:

| Scenario | How to Simulate | What to Verify |
|----------|----------------|----------------|
| Slow consumer | Respond with 200 after 25s delay | Sender respects timeout >30s; does not mark as failed prematurely |
| Consumer down | Return 503 for first 3 deliveries | Sender retries with exponential backoff; check `X-Retry-Count` |
| Duplicate delivery | Verify same `X-Webhook-Id` arrives twice | Consumer handles idempotently — no duplicate side effects |
| Out-of-order events | Process events t2 before t1 | Consumer uses event timestamp, not arrival order, for state |
| Oversized payload | Trigger event producing >1MB payload | Sender truncates or sends reference URL instead of inline data |
| Replay attack | Accept delivery with timestamp >5min old | Consumer rejects stale deliveries to prevent replay |
