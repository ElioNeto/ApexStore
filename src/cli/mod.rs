use crate::{LsmConfig, LsmEngine};
use std::io::{self, Write};
use std::path::PathBuf;

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configurar tracing
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    println!("╔═══════════════════════════════════════════════════════╗");
    println!("║     LSM-Tree Key-Value Store - Interactive CLI       ║");
    println!("║                    Fase 1: Storage Engine             ║");
    println!("╚═══════════════════════════════════════════════════════╝\n");

    // Configuração
    let config = LsmConfig::builder()
        .dir_path(PathBuf::from("./.lsm_data"))
        .memtable_max_size(4 * 1024) // 4KB para testes
        .build()?;

    println!(
        "📂 Diretório de dados: {}",
        config.core.dir_path.display()
    );

    println!("Inicializando engine...");
    let engine = LsmEngine::new(config)?;
    println!("✓ Engine inicializado com sucesso!\n");

    print_help();
    println!();

    // REPL Loop
    loop {
        print!("lsm> ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        let parts: Vec<&str> = input.splitn(4, ' ').collect();
        let command = parts[0].to_uppercase();

        match command.as_str() {
            "SET" => {
                if parts.len() < 3 {
                    println!("❌ Uso: SET <key> <value>");
                    continue;
                }
                let key = parts[1].to_string();
                let value = parts[2].as_bytes().to_vec();

                match engine.set(key.clone(), value) {
                    Ok(_) => println!("✓ SET '{}' executado com sucesso", key),
                    Err(e) => println!("❌ Erro: {}", e),
                }
            }

            "GET" => {
                if parts.len() < 2 {
                    println!("❌ Uso: GET <key>");
                    continue;
                }
                let key = parts[1];

                match engine.get(key) {
                    Ok(Some(value)) => {
                        let value_str = String::from_utf8_lossy(&value);
                        println!("✓ '{}' = '{}'", key, value_str);
                    }
                    Ok(None) => println!("⚠ Chave '{}' não encontrada", key),
                    Err(e) => println!("❌ Erro: {}", e),
                }
            }

            "DELETE" | "DEL" => {
                if parts.len() < 2 {
                    println!("❌ Uso: DELETE <key>");
                    continue;
                }
                let key = parts[1].to_string();

                match engine.delete(key.clone()) {
                    Ok(_) => println!("✓ DELETE '{}' executado (tombstone criado)", key),
                    Err(e) => println!("❌ Erro: {}", e),
                }
            }

            "STATS" => {
                // Check if "ALL" parameter is provided
                if parts.len() > 1 && parts[1].to_uppercase() == "ALL" {
                    match engine.stats_all() {
                        Ok(stats) => {
                            match serde_json::to_string_pretty(&stats) {
                                Ok(json) => println!("{}", json),
                                Err(e) => println!("❌ Erro ao serializar JSON: {}", e),
                            }
                        }
                        Err(e) => println!("❌ Erro: {}", e),
                    }
                } else {
                    println!("{}", engine.stats());
                }
            }

            "SEARCH" => {
                if parts.len() < 2 {
                    println!("❌ Uso: SEARCH <query> [--prefix]");
                    continue;
                }
                
                let query = parts[1];
                let prefix_mode = parts.len() > 2 && parts[2] == "--prefix";
                
                let results = if prefix_mode {
                    engine.search_prefix(query)
                } else {
                    engine.search(query)
                };
                
                match results {
                    Ok(records) => {
                        if records.is_empty() {
                            println!("⚠ Nenhum registro encontrado");
                        } else {
                            println!("✓ {} registro(s) encontrado(s):\n", records.len());
                            for (key, value) in records {
                                let value_str = String::from_utf8_lossy(&value);
                                println!("  {} = {}", key, value_str);
                            }
                        }
                    }
                    Err(e) => println!("❌ Erro: {}", e),
                }
            }

            "HELP" | "?" => {
                print_help();
            }

            "CLEAR" => {
                print!("\x1B[2J\x1B[1;1H"); // Clear screen ANSI code
                println!("╔═══════════════════════════════════════════════════════╗");
                println!("║     LSM-Tree Key-Value Store - Interactive CLI       ║");
                println!("╚═══════════════════════════════════════════════════════╝\n");
            }

            "EXIT" | "QUIT" | "Q" => {
                println!("👋 Encerrando LSM-Tree CLI...");
                break;
            }

            "DEMO" => {
                run_demo(&engine)?;
            }

            "BATCH" => {
                if parts.len() >= 3 && parts[1].to_uppercase() == "SET" {
                    // BATCH SET <file>
                    let file_path = parts[2];
                    println!("Importando de {}...", file_path);
                    
                    match std::fs::read_to_string(file_path) {
                        Ok(content) => {
                            let mut count = 0;
                            let mut errors = 0;
                            
                            for (line_num, line) in content.lines().enumerate() {
                                let line = line.trim();
                                if line.is_empty() || line.starts_with('#') {
                                    continue; // Skip empty lines and comments
                                }
                                
                                if let Some((key, value)) = line.split_once('=') {
                                    let key = key.trim();
                                    let value = value.trim();
                                    
                                    match engine.set(key.to_string(), value.as_bytes().to_vec()) {
                                        Ok(_) => count += 1,
                                        Err(e) => {
                                            println!("⚠ Erro na linha {}: {}", line_num + 1, e);
                                            errors += 1;
                                        }
                                    }
                                } else {
                                    println!("⚠ Linha {} inválida (formato esperado: key=value)", line_num + 1);
                                    errors += 1;
                                }
                            }
                            
                            println!("✓ {} registro(s) importado(s)", count);
                            if errors > 0 {
                                println!("⚠ {} erro(s) encontrado(s)", errors);
                            }
                        }
                        Err(e) => println!("❌ Erro ao ler arquivo: {}", e),
                    }
                } else if parts.len() >= 2 {
                    // BATCH <count> (existing functionality)
                    let count: usize = match parts[1].parse() {
                        Ok(n) => n,
                        Err(_) => {
                            println!("❌ Count inválido");
                            continue;
                        }
                    };

                    println!("Inserindo {} registros...", count);
                    let start = std::time::Instant::now();

                    for i in 0..count {
                        let key = format!("batch:{}", i);
                        let value = format!("value_{}", i).into_bytes();
                        engine.set(key, value)?;
                    }

                    let elapsed = start.elapsed();
                    println!("✓ {} registros inseridos em {:.2?}", count, elapsed);
                    println!("  Taxa: {:.0} ops/s", count as f64 / elapsed.as_secs_f64());
                } else {
                    println!("❌ Uso: BATCH <count> | BATCH SET <file>");
                }
            }

            "SCAN" => {
                if parts.len() < 2 {
                    println!("❌ Uso: SCAN <prefix>");
                    continue;
                }
                let prefix = parts[1];
                
                // Use the search_prefix method now available
                match engine.search_prefix(prefix) {
                    Ok(records) => {
                        if records.is_empty() {
                            println!("⚠ Nenhum registro encontrado com prefixo '{}'", prefix);
                        } else {
                            println!("✓ {} registro(s) com prefixo '{}':\n", records.len(), prefix);
                            for (key, value) in records {
                                let value_str = String::from_utf8_lossy(&value);
                                println!("  {} = {}", key, value_str);
                            }
                        }
                    }
                    Err(e) => println!("❌ Erro: {}", e),
                }
            }

            "ALL" => {
                println!("Listando todos os registros...\n");
                match engine.scan() {
                    Ok(records) => {
                        if records.is_empty() {
                            println!("⚠ Banco de dados vazio");
                        } else {
                            println!("┌─────────────────────────────────────────────────┐");
                            println!("│  Chave                │  Valor                 │");
                            println!("├─────────────────────────────────────────────────┤");

                            for (key, value) in records {
                                let value_str = String::from_utf8_lossy(&value);
                                let key_display = if key.len() > 20 {
                                    format!("{}...", &key[..17])
                                } else {
                                    key.clone()
                                };
                                let value_display = if value_str.len() > 20 {
                                    format!("{}...", &value_str[..17])
                                } else {
                                    value_str.to_string()
                                };
                                println!("│  {:<20} │  {:<20} │", key_display, value_display);
                            }

                            println!("└─────────────────────────────────────────────────┘");
                        }
                    }
                    Err(e) => println!("❌ Erro ao escanear: {}", e),
                }
            }

            "KEYS" => match engine.keys() {
                Ok(keys) => {
                    if keys.is_empty() {
                        println!("⚠ Nenhuma chave encontrada");
                    } else {
                        println!("Total de chaves: {}\n", keys.len());
                        for (i, key) in keys.iter().enumerate() {
                            println!("  {}. {}", i + 1, key);
                        }
                    }
                }
                Err(e) => println!("❌ Erro: {}", e),
            },

            "COUNT" => match engine.count() {
                Ok(count) => println!("✓ Total de registros ativos: {}", count),
                Err(e) => println!("❌ Erro: {}", e),
            },

            _ => {
                println!("❌ Comando desconhecido: '{}'", command);
                println!("   Digite HELP para ver comandos disponíveis");
            }
        }
    }

    Ok(())
}

fn print_help() {
    println!("Comandos disponíveis:");
    println!("  SET <key> <value>         - Insere ou atualiza um par chave-valor");
    println!("  GET <key>                 - Recupera o valor de uma chave");
    println!("  DELETE <key>              - Remove uma chave (cria tombstone)");
    println!("  SEARCH <query> [--prefix] - Busca registros (opcionalmente por prefixo)");
    println!("  SCAN <prefix>             - Lista registros com prefixo específico");
    println!("  ALL                       - Lista todos os registros do banco");
    println!("  KEYS                      - Lista apenas as chaves");
    println!("  COUNT                     - Conta registros ativos");
    println!("  STATS [ALL]               - Exibe estatísticas (básicas ou detalhadas)");
    println!("  BATCH <count>             - Insere N registros de teste");
    println!("  BATCH SET <file>          - Importa registros de arquivo");
    println!("  DEMO                      - Executa demonstração de features");
    println!("  CLEAR                     - Limpa a tela");
    println!("  HELP ou ?                 - Exibe esta ajuda");
    println!("  EXIT, QUIT ou Q           - Sai do programa");
}

fn run_demo(engine: &LsmEngine) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n╔═══════════════════════════════════════════════════════╗");
    println!("║                  DEMO AUTOMÁTICA                      ║");
    println!("╚═══════════════════════════════════════════════════════╝\n");

    println!("1. Inserindo dados de exemplo...");
    engine.set("user:alice".to_string(), b"Alice Silva".to_vec())?;
    engine.set("user:bob".to_string(), b"Bob Santos".to_vec())?;
    engine.set("user:charlie".to_string(), b"Charlie Costa".to_vec())?;
    println!("   ✓ 3 usuários inseridos\n");

    println!("2. Lendo dados...");
    if let Some(v) = engine.get("user:alice")? {
        println!("   user:alice = {}", String::from_utf8_lossy(&v));
    }
    if let Some(v) = engine.get("user:bob")? {
        println!("   user:bob = {}", String::from_utf8_lossy(&v));
    }
    println!();

    println!("3. Atualizando user:alice...");
    engine.set("user:alice".to_string(), b"Alice Silva Santos".to_vec())?;
    if let Some(v) = engine.get("user:alice")? {
        println!(
            "   user:alice = {} (atualizado)",
            String::from_utf8_lossy(&v)
        );
    }
    println!();

    println!("4. Deletando user:bob...");
    engine.delete("user:bob".to_string())?;
    match engine.get("user:bob")? {
        Some(_) => println!("   ❌ Erro: ainda existe"),
        None => println!("   ✓ user:bob deletado com sucesso"),
    }
    println!();

    println!("5. Forçando múltiplas escritas para flush...");
    for i in 0..10 {
        engine.set(
            format!("product:{}", i),
            format!(
                "Product {} - Descrição longa para forçar flush automático",
                i
            )
            .into_bytes(),
        )?;
    }
    println!("   ✓ 10 produtos inseridos\n");

    println!("6. Testando novos comandos...");
    println!("   - SEARCH user:");
    match engine.search("user:") {
        Ok(results) => println!("     Encontrados {} registros", results.len()),
        Err(e) => println!("     Erro: {}", e),
    }
    
    println!("   - SEARCH user: --prefix");
    match engine.search_prefix("user:") {
        Ok(results) => println!("     Encontrados {} registros", results.len()),
        Err(e) => println!("     Erro: {}", e),
    }
    println!();

    println!("7. Estatísticas finais (básicas):");
    println!("{}", engine.stats());
    
    println!("\n8. Estatísticas detalhadas:");
    match engine.stats_all() {
        Ok(stats) => {
            if let Ok(json) = serde_json::to_string_pretty(&stats) {
                println!("{}", json);
            }
        }
        Err(e) => println!("   Erro: {}", e),
    }

    println!("\n╔═══════════════════════════════════════════════════════╗");
    println!("║               DEMO CONCLUÍDA                          ║");
    println!("╚═══════════════════════════════════════════════════════╝\n");

    Ok(())
}
