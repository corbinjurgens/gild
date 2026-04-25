use crate::app::{App, AuthorStats};
use crate::fmt::{fmt_date, Sep};
use anyhow::Result;
use std::fs;
use std::io::Write;
use std::path::Path;

pub fn export(app: &App, format: &str, output: Option<&Path>) -> Result<()> {
    let content = match format {
        "json" => export_json(app)?,
        "csv" => export_csv(app),
        "html" => export_html(app),
        _ => anyhow::bail!("Unknown export format: {format}. Use json, csv, or html."),
    };

    match output {
        Some(path) => {
            fs::write(path, &content)?;
            eprintln!("  Exported to {}", path.display());
        }
        None => {
            let stdout = std::io::stdout();
            let mut handle = stdout.lock();
            handle.write_all(content.as_bytes())?;
        }
    }

    Ok(())
}

fn export_json(app: &App) -> Result<String> {
    let authors: Vec<serde_json::Value> = app
        .sorted_authors()
        .enumerate()
        .map(|(i, a)| author_to_json(i + 1, a))
        .collect();

    let doc = serde_json::json!({
        "repository": app.data.repo_info.name,
        "branch": app.data.repo_info.branch,
        "time_range": app.time_label(),
        "total_commits": app.cache.filtered_commits,
        "total_authors": app.cache.authors.len(),
        "authors": authors,
    });

    Ok(serde_json::to_string_pretty(&doc)?)
}

fn author_to_json(rank: usize, a: &AuthorStats) -> serde_json::Value {
    let mut obj = serde_json::json!({
        "rank": rank,
        "name": a.display_name,
        "commits": a.commits,
        "lines_added": a.lines_added,
        "lines_removed": a.lines_removed,
        "net_lines": a.lines_added as i64 - a.lines_removed as i64,
        "files_changed": a.files_changed,
        "impact": (a.impact * 10.0).round() / 10.0,
        "first_commit": fmt_date(a.first_commit, "%Y-%m-%d"),
        "last_commit": fmt_date(a.last_commit, "%Y-%m-%d"),
    });

    if a.ownership_lines > 0 {
        obj["ownership_lines"] = serde_json::json!(a.ownership_lines);
        obj["ownership_pct"] = serde_json::json!((a.ownership_pct * 10.0).round() / 10.0);
    }

    let ct = &a.change_types;
    obj["change_types"] = serde_json::json!({
        "feature": ct.feature,
        "refactor": ct.refactor,
        "rename": ct.rename,
        "trivial": ct.trivial,
        "merge": ct.merge,
        "new_files": ct.new_files,
        "deleted_files": ct.deleted_files,
        "renamed_files": ct.renamed_files,
        "whitespace_lines": ct.whitespace_lines,
    });

    obj
}

fn export_csv(app: &App) -> String {
    let mut out = String::from(
        "Rank,Author,Commits,Lines Added,Lines Removed,Net Lines,Files Changed,Impact,Ownership Lines,Ownership %,Feature,Refactor,Rename,Trivial,Merge,New Files,Deleted Files,Renamed Files,Whitespace Lines,First Commit,Last Commit\n",
    );

    for (i, a) in app.sorted_authors().enumerate() {
        let net = a.lines_added as i64 - a.lines_removed as i64;
        let ct = &a.change_types;
        out.push_str(&format!(
            "{},\"{}\",{},{},{},{},{},{:.1},{},{:.1},{},{},{},{},{},{},{},{},{},{},{}\n",
            i + 1,
            a.display_name.replace('"', "\"\""),
            a.commits,
            a.lines_added,
            a.lines_removed,
            net,
            a.files_changed,
            a.impact,
            a.ownership_lines,
            a.ownership_pct,
            ct.feature,
            ct.refactor,
            ct.rename,
            ct.trivial,
            ct.merge,
            ct.new_files,
            ct.deleted_files,
            ct.renamed_files,
            ct.whitespace_lines,
            fmt_date(a.first_commit, "%Y-%m-%d"),
            fmt_date(a.last_commit, "%Y-%m-%d"),
        ));
    }

    out
}

fn export_html(app: &App) -> String {
    let max_impact = app
        .sorted_authors()
        .next()
        .map(|a| a.impact)
        .unwrap_or(1.0)
        .max(1.0);

    let mut rows = String::new();
    for (i, a) in app.sorted_authors().enumerate() {
        let rank = i + 1;
        let net = a.lines_added as i64 - a.lines_removed as i64;
        let net_class = if net >= 0 { "added" } else { "removed" };
        let pct = (a.impact / max_impact * 100.0).max(0.0);
        let rank_class = match rank {
            1 => "gold",
            2 => "silver",
            3 => "bronze",
            _ => "",
        };
        let ownership = if a.ownership_lines > 0 {
            format!("{} ({:.1}%)", Sep(a.ownership_lines), a.ownership_pct)
        } else {
            String::from("-")
        };

        rows.push_str(&format!(
            r#"<tr>
  <td class="{rank_class}">{rank}</td>
  <td class="{rank_class}">{name}</td>
  <td>{commits}</td>
  <td class="added">+{added}</td>
  <td class="removed">-{removed}</td>
  <td class="{net_class}">{net:+}</td>
  <td>{files}</td>
  <td class="impact">{impact:.1}</td>
  <td>{ownership}</td>
  <td><div class="bar" style="width:{pct:.0}%"></div></td>
</tr>
"#,
            name = html_escape(&a.display_name),
            commits = Sep(a.commits),
            added = Sep(a.lines_added),
            removed = Sep(a.lines_removed),
            files = Sep(a.files_changed),
            impact = a.impact,
        ));
    }

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>gild — {repo} ({branch})</title>
<style>
:root {{
  --bg: #282a36; --fg: #f8f8f2; --comment: #6272a4;
  --cyan: #8be9fd; --green: #50fa7b; --orange: #ffb86c;
  --pink: #ff79c6; --purple: #bd93f9; --red: #ff5555; --yellow: #f1fa8c;
  --gold: #ffd700; --silver: #c0c0c0; --bronze: #cd7f32;
}}
* {{ margin:0; padding:0; box-sizing:border-box; }}
body {{ background:var(--bg); color:var(--fg); font-family:'SF Mono',Consolas,'Courier New',monospace; padding:2rem; }}
h1 {{ color:var(--purple); font-size:1.4rem; margin-bottom:0.3rem; }}
.meta {{ color:var(--comment); margin-bottom:1.5rem; font-size:0.9rem; }}
table {{ width:100%; border-collapse:collapse; font-size:0.85rem; }}
th {{ color:var(--comment); text-align:left; padding:0.5rem 0.7rem; border-bottom:2px solid var(--comment); }}
td {{ padding:0.4rem 0.7rem; border-bottom:1px solid #44475a; }}
tr:hover {{ background:#44475a; }}
.gold {{ color:var(--gold); font-weight:bold; }}
.silver {{ color:var(--silver); font-weight:bold; }}
.bronze {{ color:var(--bronze); font-weight:bold; }}
.added {{ color:var(--green); }}
.removed {{ color:var(--red); }}
.impact {{ color:var(--cyan); }}
.bar {{ height:14px; background:var(--purple); border-radius:2px; min-width:2px; }}
</style>
</head>
<body>
<h1>gild</h1>
<p class="meta">{repo} on {branch} &middot; {commits} commits &middot; {authors} authors &middot; {time}</p>
<table>
<tr><th>#</th><th>Author</th><th>Commits</th><th>+Lines</th><th>-Lines</th><th>Net</th><th>Files</th><th>Impact</th><th>Ownership</th><th>Share</th></tr>
{rows}
</table>
</body>
</html>"#,
        repo = html_escape(&app.data.repo_info.name),
        branch = html_escape(&app.data.repo_info.branch),
        commits = Sep(app.cache.filtered_commits),
        authors = app.cache.authors.len(),
        time = html_escape(&app.time_label()),
    )
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}
