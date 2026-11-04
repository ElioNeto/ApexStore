# 🔐 Authentication Guide

ApexStore API supports Bearer Token authentication to protect endpoints from unauthorized access.

## Overview

- **Type**: Bearer Token
- **Algorithm**: SHA-256 hashing
- **Storage**: Private, durable snapshot at `<DATA_DIR>/.apex-auth/tokens.json`; only token hashes and metadata are stored
- **Permissions**: Read, Write, Delete, Admin

## Quick Start

### 1. Enable Authentication

Set in `.env`:
```bash
API_AUTH_ENABLED=true
API_TOKEN_EXPIRY_DAYS=30
```

### 2. Create an Administrator Offline

Stop the server before using the CLI against its database directory. Create the first administrator locally and save the printed token securely:

```bash
apexstore-cli --db ./apexstore_data token create local-admin --permissions admin
```

Use the same directory as the server's `DATA_DIR`. The CLI and server share the private credential store; a database lock prevents simultaneous offline administration and server access.

### 3. Start the Server

```bash
DATA_DIR=./apexstore_data cargo run --features api --bin apexstore-server
```

### 4. Create an Application Token

**Request:**
```bash
curl -X POST http://localhost:8080/admin/tokens \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "production-api",
    "permissions": ["Read", "Write"],
    "expires_in_days": 30
  }'
```

**Response:**
```json
{
  "success": true,
  "message": "Token created successfully",
  "data": {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "name": "production-api",
    "token": "apx_a1b2c3d4e5f6...",
    "expires_at": 1709740800000000000,
    "permissions": ["Read", "Write"]
  }
}
```

⚠️ **Important**: Save the token immediately! It will not be shown again.

### 5. Use the Token

Include the token in the `Authorization` header:

```bash
curl -X POST http://localhost:8080/keys \
  -H "Authorization: Bearer apx_a1b2c3d4e5f6..." \
  -H "Content-Type: application/json" \
  -d '{"key": "user:1", "value": "Alice"}'
```

## Credential Storage and Upgrades

Credentials are isolated from the public LSM keyspace. Each update writes and syncs a temporary snapshot, atomically replaces the credential file, then syncs its directory on Unix. The directory is mode `0700` and the snapshot mode `0600` on Unix. Token hashes and metadata are protected by filesystem permissions; this file is not encrypted by SSTable/WAL encryption. Include `.apex-auth` when backing up or relocating the complete data directory, and restrict access to that backup. An LSM-only snapshot does not include credentials.

Loading malformed, unsupported or inaccessible credential storage fails server startup. It never falls back to an empty credential set. Concurrent owners of the credential store are rejected. `TokenManager::new_with_engine` now returns `Result<TokenManager, AuthError>`; library callers must handle the error.

Versions that stored credentials as `__token:*` records allowed public data writes to impersonate administrators. Those legacy records cannot be distinguished from forged credentials and are **never imported automatically**. Before upgrading an existing deployment:

1. Stop the server and retain a backup for investigation/recovery.
2. Use the upgraded offline CLI against the same database directory to create a replacement administrator, as shown above.
3. Restart with authentication enabled, create replacement application tokens and update clients. Old public-keyspace tokens no longer authenticate.

Do not copy legacy token JSON into the private file. Public key-value routes reject keys beginning with `__` (including `__token:`, `__ttl:`, `__index:` and `__idx:`), and public scans/searches hide legacy metadata. Batch and transaction key operations enforce the same boundary. Prefix deletion also rejects `_`, because that range includes reserved keys. GraphQL returns no reserved-key value and refuses reserved-key mutations.

## Permission Levels

| Permission | Description | Grants Access To |
|------------|-------------|------------------|
| `Read` | Read-only access | GET endpoints |
| `Write` | Write access; grant Read separately when needed | PUT, POST and mutation endpoints |
| `Delete` | Delete access; grant Read separately when needed | DELETE endpoints |
| `Admin` | Full access | All endpoints including `/admin/*` |

## API Endpoints

### Token Management

#### Create Token
```bash
POST /admin/tokens
Content-Type: application/json

{
  "name": "my-token",
  "permissions": ["Read", "Write"],
  "expires_in_days": 30  # Optional
}
```

#### List All Tokens
```bash
GET /admin/tokens
Authorization: Bearer <admin-token>
```

**Response:**
```json
{
  "success": true,
  "message": "2 tokens found",
  "data": {
    "tokens": [
      {
        "id": "...",
        "name": "production-api",
        "created_at": 1709740800000000000,
        "expires_at": 1712419200000000000,
        "permissions": ["Read", "Write"]
      }
    ]
  }
}
```

#### Delete Token
```bash
DELETE /admin/tokens/{id}
Authorization: Bearer <admin-token>
```

## Public Endpoints

These endpoints do NOT require authentication:

- `GET /health/liveness` — other health endpoints require authentication

## Protected Endpoints

All other endpoints require valid Bearer token when authentication is enabled:

- `GET /stats` - Requires: Read
- `GET /stats/all` - Requires: Read
- `GET /keys` - Requires: Read
- `GET /keys/{key}` - Requires: Read
- `GET /keys/search` - Requires: Read
- `GET /scan` - Requires: Read
- `POST /keys` - Requires: Write
- `POST /keys/batch` - Requires: Write
- `DELETE /keys/{key}` - Requires: Delete
- `GET /features` - Requires: Admin
- `POST /features/{name}` - Requires: Admin
- `POST /admin/tokens` - Requires: Admin
- `GET /admin/tokens` - Requires: Admin
- `DELETE /admin/tokens/{id}` - Requires: Admin

## Error Responses

### 401 Unauthorized
```json
{
  "error": "Missing authorization header",
  "status": 401
}
```

### 401 Unauthorized - Invalid Token
```json
{
  "error": "Invalid authentication token",
  "status": 401
}
```

### 401 Unauthorized - Expired Token
```json
{
  "error": "Token has expired",
  "status": 401
}
```

### 403 Forbidden
```json
{
  "error": "Insufficient permissions",
  "status": 403
}
```

## Security Best Practices

### 1. Token Storage

- ✅ Store tokens in environment variables
- ✅ Use secrets management (e.g., AWS Secrets Manager, HashiCorp Vault)
- ❌ Never commit tokens to version control
- ❌ Never log tokens in plain text

### 2. Token Generation

- Tokens are 32-byte cryptographically secure random values
- Prefix: `apx_` for easy identification
- Stored as SHA-256 hash (one-way)

### 3. Token Comparison

- Uses constant-time comparison to prevent timing attacks
- Validates hash, not plain token

### 4. HTTPS/TLS

⚠️ **Always use HTTPS in production!**

```bash
# Set up reverse proxy with Nginx/Caddy
# Or use Railway/Fly.io for automatic HTTPS
```

### 5. Token Rotation

- Set reasonable expiry times (30-90 days)
- Delete unused tokens regularly
- Rotate tokens after suspected compromise

## Migration Path

### Phase 1: Optional Authentication (Current)

Authentication is **disabled by default** for backward compatibility.

```bash
API_AUTH_ENABLED=false  # Default
```

### Phase 2: Enable in Production

Enable authentication for your deployment:

```bash
API_AUTH_ENABLED=true
```

### Phase 3: Future (v2.0)

Authentication will be **enabled by default** in v2.0.

## Examples

### Node.js/JavaScript

```javascript
const axios = require('axios');

const client = axios.create({
  baseURL: 'http://localhost:8080',
  headers: {
    'Authorization': `Bearer ${process.env.APEXSTORE_TOKEN}`
  }
});

// Use the client
await client.post('/keys', {
  key: 'user:1',
  value: 'Alice'
});
```

### Python

```python
import requests
import os

token = os.getenv('APEXSTORE_TOKEN')

headers = {
    'Authorization': f'Bearer {token}',
    'Content-Type': 'application/json'
}

response = requests.post(
    'http://localhost:8080/keys',
    json={'key': 'user:1', 'value': 'Alice'},
    headers=headers
)
```

### cURL

```bash
# Store token in variable
TOKEN="apx_a1b2c3d4e5f6..."

# Make requests
curl -X GET "http://localhost:8080/keys/user:1" \
  -H "Authorization: Bearer $TOKEN"
```

## Troubleshooting

### "Missing authorization header"

- Ensure you're including the `Authorization` header
- Format: `Authorization: Bearer <token>`

### "Invalid authentication token"

- Check token is correct (copy-paste errors)
- Verify token wasn't deleted
- Confirm server restart (tokens are in-memory)

### "Token has expired"

- Token exceeded expiry time
- Create a new token

### "Insufficient permissions"

- Token doesn't have required permission
- Create token with correct permissions

## Future Enhancements

- [ ] Persistent token storage (database)
- [ ] JWT support
- [ ] OAuth2 integration
- [ ] Rate limiting per token
- [ ] Audit logging
- [ ] Token scopes (granular permissions)

---

**Security Contact**: For security issues, please email security@apexstore.io (or create a private issue)
