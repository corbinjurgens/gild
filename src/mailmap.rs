use crate::util::load_or_default;
use std::convert::Infallible;
use std::path::Path;

pub struct MailmapEntry {
    pub canonical_name: Option<String>,
    pub canonical_email: String,
    pub commit_name: Option<String>,
    pub commit_email: Option<String>,
}

pub fn load(repo_path: &Path) -> Vec<MailmapEntry> {
    load_or_default(&repo_path.join(".mailmap"), |s| {
        Ok::<_, Infallible>(parse(s))
    })
}

fn parse(content: &str) -> Vec<MailmapEntry> {
    content
        .lines()
        .filter(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with('#')
        })
        .filter_map(parse_line)
        .collect()
}

fn parse_line(line: &str) -> Option<MailmapEntry> {
    let line = line.trim();
    let emails = find_emails(line);

    match emails.len() {
        1 => {
            let (start, _, email) = &emails[0];
            let name = line[..*start].trim();
            Some(MailmapEntry {
                canonical_name: non_empty(name),
                canonical_email: email.to_string(),
                commit_name: None,
                commit_email: None,
            })
        }
        2 => {
            let (s1, e1, email1) = &emails[0];
            let (s2, _, email2) = &emails[1];
            let name_before = line[..*s1].trim();
            let name_between = line[*e1..*s2].trim();

            if name_between.is_empty() {
                Some(MailmapEntry {
                    canonical_name: non_empty(name_before),
                    canonical_email: email1.to_string(),
                    commit_name: None,
                    commit_email: Some(email2.to_string()),
                })
            } else {
                Some(MailmapEntry {
                    canonical_name: non_empty(name_before),
                    canonical_email: email1.to_string(),
                    commit_name: Some(name_between.to_string()),
                    commit_email: Some(email2.to_string()),
                })
            }
        }
        _ => None,
    }
}

fn find_emails(s: &str) -> Vec<(usize, usize, String)> {
    let mut results = Vec::new();
    let mut pos = 0;
    while let Some(open) = s[pos..].find('<') {
        let abs_open = pos + open;
        if let Some(close) = s[abs_open..].find('>') {
            let abs_close = abs_open + close;
            let email = s[abs_open + 1..abs_close].to_string();
            results.push((abs_open, abs_close + 1, email));
            pos = abs_close + 1;
        } else {
            break;
        }
    }
    results
}

fn non_empty(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}
