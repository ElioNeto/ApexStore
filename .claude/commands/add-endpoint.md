# Comando: /add-endpoint

Adiciona um novo endpoint REST à API Actix-Web.

## Uso
```
/add-endpoint <METHOD> <path> <descrição>
```

## Exemplo
```
/add-endpoint DELETE /keys/{key} "Remove uma chave do store"
```

## Passos a seguir

1. **Handler** — criar ou adicionar em `src/api/`:
```rust
pub async fn delete_key(
    path: web::Path<String>,
    engine: web::Data<Arc<RwLock<LsmEngine>>>,
) -> impl Responder {
    let key = path.into_inner();
    // lógica...
}
```

2. **Registrar** no builder de rotas com `.route("...", web::delete().to(delete_key))`

3. **Engine** — se o endpoint precisar de nova operação no Engine, adicionar método em `src/core/engine.rs` seguindo o padrão de lock:
```rust
pub fn delete(&self, key: &str) -> Result<(), ApexError> {
    let mut guard = self.inner.write();
    // ...
}
```

4. **Frontend** — adicionar método no `ApexStoreService` em `frontend/src/app/services/apex-store.service.ts`

5. **Stats** — se o endpoint gera métricas, expor em `GET /stats/all`

## Padrão de resposta
- Sucesso: `200 OK` com JSON ou `204 No Content`
- Chave não encontrada: `404 Not Found` com `{"error": "Key not found"}`
- Erro interno: `500` com `{"error": "<mensagem>"}`
