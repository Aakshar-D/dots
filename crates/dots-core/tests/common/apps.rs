//! A small app for the app library's tests: Expenses, as the spec's review mockup shows it.

use std::path::Path;

pub const MANIFEST: &str = r#"{
  "name": "expenses",
  "title": "Expenses",
  "description": "Logs expenses and totals them by month",
  "tools": [
    {
      "name": "add_expense",
      "description": "Log an expense",
      "input": {
        "type": "object",
        "properties": {
          "item": { "type": "string", "minLength": 1, "maxLength": 80 },
          "amount": { "type": "number", "minimum": 0 },
          "on": { "type": "string", "description": "YYYY-MM-DD" }
        },
        "required": ["item", "amount", "on"],
        "additionalProperties": false
      },
      "writes": true
    },
    {
      "name": "list_expenses",
      "description": "The expenses of a month, oldest first",
      "input": {
        "type": "object",
        "properties": { "month": { "type": "string", "description": "YYYY-MM" } },
        "required": ["month"]
      }
    },
    {
      "name": "month_total",
      "description": "The total of a month",
      "input": {
        "type": "object",
        "properties": { "month": { "type": "string" } },
        "required": ["month"]
      }
    }
  ]
}"#;

pub const LOGIC: &str = r#"
export function add_expense({ item, amount, on }, ctx) {
  const id = ctx.id();
  ctx.db.exec("INSERT INTO expenses (id, item, amount, on_day, added_at, added_by) VALUES (?, ?, ?, ?, ?, ?)",
    [id, item, amount, on, ctx.now, ctx.actor.kind === "dot" ? ctx.actor.name : "you"]);
  return { id };
}

export function list_expenses({ month }, ctx) {
  return ctx.db.query("SELECT id, item, amount, on_day AS \"on\" FROM expenses WHERE substr(on_day, 1, 7) = ? ORDER BY on_day, id", [month]);
}

export function month_total({ month }, ctx) {
  return list_expenses({ month }, ctx).reduce((sum, e) => sum + e.amount, 0);
}
"#;

pub const MIGRATION: &str = "CREATE TABLE expenses (id TEXT PRIMARY KEY, item TEXT NOT NULL, \
    amount REAL NOT NULL, on_day TEXT NOT NULL, added_at TEXT NOT NULL, added_by TEXT NOT NULL);\n";

pub const SAMPLE: &str = "INSERT INTO expenses VALUES ('s1', 'Coffee', 4.5, '2026-10-22', \
    '2026-10-22T09:00:00+05:30', 'you');\nINSERT INTO expenses VALUES ('s2', 'Train', 12, \
    '2026-10-22', '2026-10-22T09:05:00+05:30', 'you');\n";

pub const UI: &str = "<!doctype html><title>Expenses</title><div id=app></div>\
    <script src=\"app.js\"></script>\n";

pub const UI_SCRIPT: &str = "dots.call('list_expenses', { month: '2026-10' }).then(rows => \
    { document.getElementById('app').textContent = rows.length + ' expenses'; });\n";

pub const TEST: &str = r#"
test("a month's total adds its expenses", () => {
  call("add_expense", { item: "Coffee", amount: 4.5, on: "2026-10-22" });
  call("add_expense", { item: "Train", amount: 12, on: "2026-10-23" });
  call("add_expense", { item: "Rent", amount: 900, on: "2026-11-01" });
  assert.equal(call("month_total", { month: "2026-10" }), 16.5);
});

test("a dot's expense names the dot", () => {
  call("add_expense", { item: "Ink", amount: 3, on: "2026-10-01" }, { actor: { kind: "dot", id: "d1", name: "builder" } });
  assert.deepEqual(call("list_expenses", { month: "2026-10" }).map(e => e.item), ["Ink"]);
});
"#;

/// The Expenses app's files: (path, text).
pub fn expenses() -> Vec<(&'static str, String)> {
    vec![
        ("app.json", MANIFEST.to_string()),
        ("logic.js", LOGIC.to_string()),
        ("ui/index.html", UI.to_string()),
        ("ui/app.js", UI_SCRIPT.to_string()),
        ("migrations/0001_expenses.sql", MIGRATION.to_string()),
        ("sample.sql", SAMPLE.to_string()),
        ("tests/expenses.test.js", TEST.to_string()),
    ]
}

/// Writes `files` below `dir`.
pub fn write_app(dir: &Path, files: &[(&str, String)]) {
    for (path, text) in files {
        let file = dir.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
}

/// The Expenses app with `path` replaced by `text`, or left out when `text` is `None`.
pub fn expenses_with(path: &str, text: Option<&str>) -> Vec<(&'static str, String)> {
    let mut files: Vec<(&'static str, String)> =
        expenses().into_iter().filter(|(p, _)| *p != path).collect();
    if let Some(text) = text {
        files.push((
            Box::leak(path.to_string().into_boxed_str()),
            text.to_string(),
        ));
    }
    files
}
