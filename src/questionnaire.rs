use crate::identity::IdentityGroup;
use crate::identity_map::{format_identity, IdentityMap};
use std::io::{self, IsTerminal, Write};
use std::path::Path;

pub fn run(
    groups: &[IdentityGroup],
    identity_map: &mut IdentityMap,
    map_path: &Path,
    force: bool,
) -> bool {
    if !force && !io::stdin().is_terminal() {
        return false;
    }

    let candidates = find_candidates(groups, identity_map);
    if candidates.is_empty() {
        return false;
    }

    eprintln!(
        "\n  {} potential identity match(es). Press [q] to stop.\n",
        candidates.len()
    );

    let mut changed = false;

    for (idx, &(gi, gj)) in candidates.iter().enumerate() {
        eprintln!(
            "  \x1b[1m({}/{})\x1b[0m Same person?",
            idx + 1,
            candidates.len()
        );
        for (n, e) in &groups[gi].aliases {
            eprintln!("    \x1b[36m{}\x1b[0m", format_identity(n, e));
        }
        eprintln!("    \x1b[90mvs\x1b[0m");
        for (n, e) in &groups[gj].aliases {
            eprintln!("    \x1b[33m{}\x1b[0m", format_identity(n, e));
        }
        eprint!("  \x1b[1m[y]es  [n]o  [d]on't know  [s]kip (ask again next run)  [q]uit >\x1b[0m ");
        io::stderr().flush().ok();

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            break;
        }

        match input.trim().to_lowercase().chars().next() {
            Some('y') => {
                identity_map.add_merge(&groups[gi].aliases, &groups[gj].aliases);
                changed = true;
                eprintln!("    \x1b[32mMerged.\x1b[0m\n");
            }
            Some('n') => {
                identity_map.add_reject(&groups[gi].aliases, &groups[gj].aliases);
                changed = true;
                eprintln!("    \x1b[31mRejected.\x1b[0m\n");
            }
            Some('d') => {
                identity_map.add_unsure(&groups[gi].aliases, &groups[gj].aliases);
                changed = true;
                eprintln!("    \x1b[33mMarked unsure.\x1b[0m\n");
            }
            Some('q') => {
                eprintln!();
                break;
            }
            _ => {
                eprintln!("    \x1b[90mSkipped (will ask again next run).\x1b[0m\n");
            }
        }
    }

    if changed {
        if let Err(e) = identity_map.save(map_path) {
            eprintln!("  Warning: failed to save identity map: {}", e);
        }
    }

    changed
}

fn find_candidates(
    groups: &[IdentityGroup],
    identity_map: &IdentityMap,
) -> Vec<(usize, usize)> {
    let mut candidates = Vec::new();

    for i in 0..groups.len() {
        for j in (i + 1)..groups.len() {
            if is_already_handled(identity_map, &groups[i], &groups[j]) {
                continue;
            }
            if let Some(score) = similarity_score(&groups[i], &groups[j]) {
                candidates.push((i, j, score));
            }
        }
    }

    candidates.sort_by_key(|c| std::cmp::Reverse(c.2));
    candidates.into_iter().map(|(i, j, _)| (i, j)).collect()
}

fn is_already_handled(map: &IdentityMap, a: &IdentityGroup, b: &IdentityGroup) -> bool {
    for (na, ea) in &a.aliases {
        let fa = format_identity(na, ea);
        for (nb, eb) in &b.aliases {
            let fb = format_identity(nb, eb);
            if map.is_rejected(&fa, &fb) || map.is_unsure(&fa, &fb) {
                return true;
            }
        }
    }

    for (na, ea) in &a.aliases {
        let fa = format_identity(na, ea);
        if let Some(ga) = map.find_group_for_member(&fa) {
            for (nb, eb) in &b.aliases {
                let fb = format_identity(nb, eb);
                if map.find_group_for_member(&fb) == Some(ga) {
                    return true;
                }
            }
        }
    }

    false
}

fn similarity_score(a: &IdentityGroup, b: &IdentityGroup) -> Option<usize> {
    let mut best = 0usize;

    for (na, ea) in &a.aliases {
        let norm_a = na.trim().to_lowercase();
        let local_a = ea.split('@').next().unwrap_or("").to_lowercase();
        let domain_a = ea.split('@').nth(1).unwrap_or("").to_lowercase();

        for (nb, eb) in &b.aliases {
            let norm_b = nb.trim().to_lowercase();
            let local_b = eb.split('@').next().unwrap_or("").to_lowercase();
            let domain_b = eb.split('@').nth(1).unwrap_or("").to_lowercase();

            if norm_a == norm_b && !norm_a.is_empty() {
                best = best.max(100);
                continue;
            }

            if local_a == local_b && !local_a.is_empty() && local_a.len() >= 3 {
                best = best.max(90);
            }

            if !norm_a.is_empty() && !norm_b.is_empty() {
                if name_contains_word(&norm_a, &norm_b) || name_contains_word(&norm_b, &norm_a) {
                    let score = if domain_a == domain_b { 80 } else { 60 };
                    best = best.max(score);
                }

                let lev = strsim::levenshtein(&norm_a, &norm_b);
                let max_len = norm_a.chars().count().max(norm_b.chars().count());
                if max_len > 0 && lev <= max_len / 3 && lev <= 5 {
                    let score = if domain_a == domain_b { 70 } else { 50 };
                    best = best.max(score);
                }
            }

            if !local_a.is_empty() && local_a.len() >= 3 {
                if norm_b.contains(&local_a) || local_a.contains(&norm_b) {
                    best = best.max(55);
                }
            }
            if !local_b.is_empty() && local_b.len() >= 3 {
                if norm_a.contains(&local_b) || local_b.contains(&norm_a) {
                    best = best.max(55);
                }
            }
        }
    }

    if best >= 50 {
        Some(best)
    } else {
        None
    }
}

fn name_contains_word(haystack: &str, needle_name: &str) -> bool {
    needle_name
        .split_whitespace()
        .any(|word| word.len() >= 3 && haystack.contains(word))
}
