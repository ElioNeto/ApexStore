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
  start_key  string  opcional
  end_key    string  opcional
  limit      int     default 1000, max 10000
  cursor     string  opcional

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

## Regras de contrato

1. Nunca retornar 200 com erro no body
2. Corpo de erro padrão: `{ "error": "mensagem" }`
3. Paginação sempre cursor-based
4. `next_cursor: null` indica última página — nunca omitir
5. Limite máximo 10000 — acima disso 429
6. Keys são case-sensitive
