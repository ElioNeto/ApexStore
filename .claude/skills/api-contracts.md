# Skill: API Contracts (ApexStore REST)

Referência dos contratos HTTP do ApexStore. Carregue ao trabalhar com `src/api/`, testes de integração HTTP ou documentação de endpoints.

---

## Endpoints existentes

### POST /keys
```
Request:  { "key": string, "value": string }
Response: 201 { "status": "ok" }
Errors:   400 key/value ausente | 500 engine error
```

### GET /keys/{key}
```
Response: 200 { "value": string }
Errors:   404 chave não encontrada | 500 engine error
```

### DELETE /keys/{key}
```
Response: 200 { "status": "deleted" }
Errors:   404 | 500
```

### GET /scan
```
Query params:
  start_key  string  opcional — início do range (inclusivo)
  end_key    string  opcional — fim do range (exclusivo)
  limit      int     opcional — default 1000, max 10000
  cursor     string  opcional — token da página anterior

Response: 200 {
  "items": [ { "key": string, "value": string } ],
  "next_cursor": string | null,
  "count": int
}
Errors: 400 parâmetros inválidos | 429 limit > MAX
```

### GET /keys/search
```
Query params:
  q       string  prefixo de busca
  limit   int     opcional — default 100
  cursor  string  opcional

Response: 200 {
  "keys": [ string ],
  "next_cursor": string | null
}
Errors: 400 q ausente
```

### GET /stats/all
```
Response: 200 {
  "memory": { "entries": int, "size_bytes": int },
  "wal":    { "entries": int, "size_bytes": int },
  "disk":   { "sstables": int, "total_bytes": int },
  "bloom":  { "fpr": float },
  "cache":  { "hits": int, "misses": int, "hit_rate": float }
}
```

---

## Autenticação

Quando `AUTH_ENABLED=true`, todos os endpoints requerem:
```
Authorization: Bearer <token>
```
Token configurado via `AUTH_TOKEN` env var. Retorna `401` se ausente/inválido.

---

## Regras de contrato

1. **Nunca retornar 200 com erro no body** — use o status HTTP correto
2. **Corpo de erro padrão**: `{ "error": "mensagem descritiva" }`
3. **Paginação sempre cursor-based** — não usar offset/page numérico
4. **`next_cursor: null`** indica última página — nunca omitir o campo
5. **Limite máximo de 10000** — acima disso retornar 429 com mensagem explicativa
6. **Keys são case-sensitive** — `"Foo"` ≠ `"foo"`

---

## Exemplo curl de smoke test

```bash
# Inserir
curl -s -X POST http://localhost:8080/keys \
  -H 'Content-Type: application/json' \
  -d '{"key":"hello","value":"world"}'

# Buscar
curl -s http://localhost:8080/keys/hello

# Range scan
curl -s 'http://localhost:8080/scan?start_key=a&end_key=z&limit=10'

# Prefix search
curl -s 'http://localhost:8080/keys/search?q=hel&limit=5'
```
