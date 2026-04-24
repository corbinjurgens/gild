use crate::identity::IdentityGroup;
use crate::identity_map::{format_identity, IdentityMap};
use std::collections::HashSet;

struct AliasNorm {
    formatted: String,
    name: String,
    local: String,
    domain: String,
}

fn normalize(aliases: &[(String, String)]) -> Vec<AliasNorm> {
    aliases
        .iter()
        .map(|(n, e)| {
            let mut parts = e.splitn(2, '@');
            let local = parts.next().unwrap_or("").to_lowercase();
            let domain = parts.next().unwrap_or("").to_lowercase();
            AliasNorm {
                formatted: format_identity(n, e),
                name: n.trim().to_lowercase(),
                local,
                domain,
            }
        })
        .collect()
}

pub fn find_candidates(
    groups: &[IdentityGroup],
    identity_map: &IdentityMap,
) -> Vec<(usize, usize)> {
    let normals: Vec<Vec<AliasNorm>> = groups.iter().map(|g| normalize(&g.aliases)).collect();

    let handled: HashSet<String> = identity_map
        .reject
        .iter()
        .chain(identity_map.unsure.iter())
        .map(|p| pair_key(&p.a, &p.b))
        .collect();

    let mut candidates = Vec::new();
    for i in 0..groups.len() {
        for j in (i + 1)..groups.len() {
            if is_already_handled(identity_map, &handled, &normals[i], &normals[j]) {
                continue;
            }
            if let Some(score) = similarity_score(&normals[i], &normals[j]) {
                candidates.push((i, j, score));
            }
        }
    }

    candidates.sort_by_key(|c| std::cmp::Reverse(c.2));
    candidates.into_iter().map(|(i, j, _)| (i, j)).collect()
}

fn pair_key(a: &str, b: &str) -> String {
    if a <= b {
        format!("{}\x00{}", a, b)
    } else {
        format!("{}\x00{}", b, a)
    }
}

fn is_already_handled(
    map: &IdentityMap,
    handled: &HashSet<String>,
    a: &[AliasNorm],
    b: &[AliasNorm],
) -> bool {
    for na in a {
        for nb in b {
            if handled.contains(&pair_key(&na.formatted, &nb.formatted)) {
                return true;
            }
        }
    }

    for na in a {
        if let Some(ga) = map.find_group_for_member(&na.formatted) {
            for nb in b {
                if map.find_group_for_member(&nb.formatted) == Some(ga) {
                    return true;
                }
            }
        }
    }

    false
}

fn similarity_score(a: &[AliasNorm], b: &[AliasNorm]) -> Option<usize> {
    let mut best = 0usize;

    for na in a {
        for nb in b {
            if !na.name.is_empty() && na.name == nb.name {
                best = best.max(100);
                continue;
            }

            if na.local.len() >= 3 && na.local == nb.local {
                best = best.max(90);
            }

            if !na.name.is_empty() && !nb.name.is_empty() {
                if name_contains_word(&na.name, &nb.name)
                    || name_contains_word(&nb.name, &na.name)
                {
                    let score = if na.domain == nb.domain { 80 } else { 60 };
                    best = best.max(score);
                }

                let lev = strsim::levenshtein(&na.name, &nb.name);
                let max_len = na.name.chars().count().max(nb.name.chars().count());
                if max_len > 0 && lev <= max_len / 3 && lev <= 5 {
                    let score = if na.domain == nb.domain { 70 } else { 50 };
                    best = best.max(score);
                }
            }

            if na.local.len() >= 3
                && (nb.name.contains(&na.local) || na.local.contains(&nb.name))
            {
                best = best.max(55);
            }
            if nb.local.len() >= 3
                && (na.name.contains(&nb.local) || nb.local.contains(&na.name))
            {
                best = best.max(55);
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
