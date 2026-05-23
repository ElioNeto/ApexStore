//! Admin dashboard — real-time monitoring and management UI.
//!
//! Provides a single `GET /admin/dashboard` endpoint that returns an embedded
//! HTML page with live engine statistics. The page auto-refreshes every 5
//! seconds using a JavaScript timer.

use crate::LsmEngine;
use actix_web::{get, web, HttpResponse, Responder};

/// Handler for `GET /admin/dashboard` — returns an HTML monitoring page.
#[get("/dashboard")]
pub async fn admin_dashboard(engine: web::Data<LsmEngine>) -> impl Responder {
    // Fetch engine stats
    let stats = engine.stats_all().unwrap_or_default();
    let column_families = {
        let core = engine.lock_core();
        core.version_set().column_families()
    };
    let compaction_running = engine.is_compaction_running();
    let metrics = engine.metrics();

    let metrics_snapshot = metrics.snapshot();

    // Build embedded HTML
    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>ApexStore Admin Dashboard</title>
  <style>
    * {{ margin: 0; padding: 0; box-sizing: border-box; }}
    body {{
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
      background: #0d1117;
      color: #c9d1d9;
      padding: 2rem;
    }}
    h1 {{ color: #58a6ff; margin-bottom: 1.5rem; font-size: 1.8rem; }}
    h2 {{ color: #8b949e; font-size: 1.1rem; margin-bottom: 0.8rem; border-bottom: 1px solid #21262d; padding-bottom: 0.3rem; }}
    .grid {{
      display: grid;
      grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
      gap: 1rem;
      margin-bottom: 2rem;
    }}
    .card {{
      background: #161b22;
      border: 1px solid #30363d;
      border-radius: 6px;
      padding: 1rem;
    }}
    .card .label {{ color: #8b949e; font-size: 0.85rem; text-transform: uppercase; letter-spacing: 0.5px; }}
    .card .value {{ font-size: 1.6rem; font-weight: 600; color: #f0f6fc; margin-top: 0.3rem; }}
    .card .value.green {{ color: #3fb950; }}
    .card .value.yellow {{ color: #d29922; }}
    .card .value.red {{ color: #f85149; }}
    .card .value.blue {{ color: #58a6ff; }}
    .status {{
      display: inline-block;
      padding: 0.2rem 0.6rem;
      border-radius: 12px;
      font-size: 0.8rem;
      font-weight: 600;
    }}
    .status.running {{ background: #1b4426; color: #3fb950; }}
    .status.idle {{ background: #1f2a3f; color: #58a6ff; }}
    .table-list {{
      list-style: none;
      margin-top: 0.5rem;
    }}
    .table-list li {{
      padding: 0.3rem 0;
      border-bottom: 1px solid #21262d;
      font-size: 0.9rem;
    }}
    .footer {{
      margin-top: 2rem;
      font-size: 0.8rem;
      color: #484f58;
      text-align: center;
    }}
    .refresh-note {{
      font-size: 0.75rem;
      color: #484f58;
      margin-bottom: 1rem;
    }}
  </style>
</head>
<body>
  <h1>⬡ ApexStore Dashboard</h1>
  <p class="refresh-note">⏱ Auto-refreshing every 5 seconds</p>

  <h2>Engine Stats</h2>
  <div class="grid">
    <div class="card">
      <div class="label">Column Families</div>
      <div class="value blue">{cf_count}</div>
    </div>
    <div class="card">
      <div class="label">SST Files</div>
      <div class="value">{sst_files}</div>
    </div>
    <div class="card">
      <div class="label">SST Size</div>
      <div class="value">{sst_kb} KB</div>
    </div>
    <div class="card">
      <div class="label">WAL Size</div>
      <div class="value">{wal_kb} KB</div>
    </div>
    <div class="card">
      <div class="label">Memtable Records</div>
      <div class="value">{mem_records}</div>
    </div>
    <div class="card">
      <div class="label">Memtable Size</div>
      <div class="value">{mem_kb} KB</div>
    </div>
    <div class="card">
      <div class="label">Total Records</div>
      <div class="value">{total_records}</div>
    </div>
    <div class="card">
      <div class="label">Max Levels Reached</div>
      <div class="value">{max_levels}</div>
    </div>
  </div>

  <h2>Compaction</h2>
  <div class="grid">
    <div class="card">
      <div class="label">Status</div>
      <div class="value"><span class="status {compact_status_class}">{compact_status}</span></div>
    </div>
    <div class="card">
      <div class="label">Compactions Completed</div>
      <div class="value">{compactions_completed}</div>
    </div>
    <div class="card">
      <div class="label">Files Merged (last)</div>
      <div class="value">{files_merged}</div>
    </div>
    <div class="card">
      <div class="label">Bytes Read (last)</div>
      <div class="value">{bytes_read}</div>
    </div>
    <div class="card">
      <div class="label">Bytes Written (last)</div>
      <div class="value">{bytes_written}</div>
    </div>
  </div>

  <h2>Operations</h2>
  <div class="grid">
    <div class="card">
      <div class="label">Sets</div>
      <div class="value">{sets}</div>
    </div>
    <div class="card">
      <div class="label">Gets</div>
      <div class="value">{gets}</div>
    </div>
    <div class="card">
      <div class="label">Deletes</div>
      <div class="value">{deletes}</div>
    </div>
    <div class="card">
      <div class="label">Scans</div>
      <div class="value">{scans}</div>
    </div>
    <div class="card">
      <div class="label">Flushes</div>
      <div class="value">{flushes}</div>
    </div>
    <div class="card">
      <div class="label">Cache Hits</div>
      <div class="value green">{cache_hits}</div>
    </div>
    <div class="card">
      <div class="label">Cache Misses</div>
      <div class="value red">{cache_misses}</div>
    </div>
    <div class="card">
      <div class="label">Bloom Negatives</div>
      <div class="value">{bloom_negatives}</div>
    </div>
    <div class="card">
      <div class="label">Errors</div>
      <div class="value red">{errors}</div>
    </div>
  </div>

  <h2>Column Families</h2>
  <div class="card">
    <ul class="table-list">
      {cf_list}
    </ul>
  </div>

  <div class="footer">
    ApexStore v{version} · Updated at <span id="updated-at"></span>
  </div>

  <script>
    function updateTime() {{
      document.getElementById('updated-at').textContent = new Date().toLocaleTimeString();
    }}
    updateTime();
    setInterval(updateTime, 1000);
    setTimeout(function() {{ location.reload(); }}, 5000);
  </script>
</body>
</html>"#,
        cf_count = column_families.len(),
        sst_files = stats.sst_files,
        sst_kb = stats.sst_kb,
        wal_kb = stats.wal_kb,
        mem_records = stats.mem_records,
        mem_kb = stats.mem_kb,
        total_records = stats.total_records,
        max_levels = stats.max_levels_reached,
        compact_status_class = if compaction_running {
            "running"
        } else {
            "idle"
        },
        compact_status = if compaction_running {
            "Running"
        } else {
            "Idle"
        },
        compactions_completed = metrics_snapshot.compactions,
        files_merged = stats.last_compaction_files_merged,
        bytes_read = stats.last_compaction_bytes_read,
        bytes_written = stats.last_compaction_bytes_written,
        sets = metrics_snapshot.sets,
        gets = metrics_snapshot.gets,
        deletes = metrics_snapshot.deletes,
        scans = metrics_snapshot.scans,
        flushes = metrics_snapshot.flushes,
        cache_hits = metrics_snapshot.cache_hits,
        cache_misses = metrics_snapshot.cache_misses,
        bloom_negatives = metrics_snapshot.bloom_filter_negatives,
        errors = metrics_snapshot.errors,
        cf_list = column_families
            .iter()
            .map(|cf| format!("<li>{}</li>", cf))
            .collect::<Vec<_>>()
            .join("\n"),
        version = env!("CARGO_PKG_VERSION"),
    );

    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(html)
}
