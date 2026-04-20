use crate::git::RawCommit;
use crate::identity_map::{parse_identity, IdentityMap};
use crate::mailmap::MailmapEntry;
use std::collections::HashMap;

pub struct IdentityGroup {
    pub display_name: String,
    pub aliases: Vec<(String, String)>,
}

pub fn merge(
    commits: &[RawCommit],
    identity_map: &IdentityMap,
    mailmap: &[MailmapEntry],
) -> (Vec<IdentityGroup>, Vec<usize>, HashMap<(String, String), usize>) {
    let mut pair_counts: HashMap<(String, String), usize> = HashMap::new();
    for c in commits {
        *pair_counts
            .entry((c.author_name.clone(), c.author_email.clone()))
            .or_insert(0) += 1;
    }

    let pairs: Vec<(String, String)> = pair_counts.keys().cloned().collect();
    let n = pairs.len();

    let pair_index: HashMap<(String, String), usize> = pairs
        .iter()
        .enumerate()
        .map(|(i, p)| (p.clone(), i))
        .collect();

    let mut parent: Vec<usize> = (0..n).collect();
    let mut rank: Vec<usize> = vec![0; n];

    for group in &identity_map.group {
        let indices: Vec<usize> = group
            .members
            .iter()
            .filter_map(|m| parse_identity(m))
            .filter_map(|p| pair_index.get(&p).copied())
            .collect();
        for w in indices.windows(2) {
            union(&mut parent, &mut rank, w[0], w[1]);
        }
    }

    let mailmap_names = apply_mailmap(&pairs, mailmap, &mut parent, &mut rank);

    let mut email_buckets: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, (_, email)) in pairs.iter().enumerate() {
        let norm = email.trim().to_lowercase();
        if !norm.is_empty() {
            email_buckets.entry(norm).or_default().push(i);
        }
    }
    for indices in email_buckets.values() {
        for w in indices.windows(2) {
            union(&mut parent, &mut rank, w[0], w[1]);
        }
    }

    let mut group_members: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        group_members
            .entry(find(&mut parent, i))
            .or_default()
            .push(i);
    }

    let mut groups: Vec<IdentityGroup> = Vec::new();
    let mut pair_to_group: HashMap<(String, String), usize> = HashMap::new();

    for members in group_members.values() {
        let group_idx = groups.len();

        let root = find(&mut parent, members[0]);
        let mailmap_name = mailmap_names.get(&root);

        let display_name = if let Some(name) = mailmap_name {
            name.clone()
        } else {
            let mut name_counts: HashMap<&str, usize> = HashMap::new();
            for &m in members {
                let count = pair_counts[&pairs[m]];
                *name_counts.entry(&pairs[m].0).or_insert(0) += count;
            }
            name_counts
                .iter()
                .max_by_key(|(_, count)| *count)
                .map(|(name, _)| name.to_string())
                .unwrap_or_else(|| "Unknown".to_string())
        };

        let aliases: Vec<(String, String)> =
            members.iter().map(|&m| pairs[m].clone()).collect();

        groups.push(IdentityGroup {
            display_name,
            aliases,
        });

        for &m in members {
            pair_to_group.insert(pairs[m].clone(), group_idx);
        }
    }

    let assignments: Vec<usize> = commits
        .iter()
        .map(|c| pair_to_group[&(c.author_name.clone(), c.author_email.clone())])
        .collect();

    (groups, assignments, pair_to_group)
}

fn apply_mailmap(
    pairs: &[(String, String)],
    mailmap: &[MailmapEntry],
    parent: &mut [usize],
    rank: &mut [usize],
) -> HashMap<usize, String> {
    let mut canonical_names: HashMap<usize, String> = HashMap::new();

    for entry in mailmap {
        let commit_email = match &entry.commit_email {
            Some(e) => e,
            None => continue,
        };

        let canonical_indices: Vec<usize> = pairs
            .iter()
            .enumerate()
            .filter(|(_, (_, e))| e.eq_ignore_ascii_case(&entry.canonical_email))
            .map(|(i, _)| i)
            .collect();

        let commit_indices: Vec<usize> = if let Some(ref cname) = entry.commit_name {
            pairs
                .iter()
                .enumerate()
                .filter(|(_, (n, e))| {
                    e.eq_ignore_ascii_case(commit_email)
                        && n.eq_ignore_ascii_case(cname)
                })
                .map(|(i, _)| i)
                .collect()
        } else {
            pairs
                .iter()
                .enumerate()
                .filter(|(_, (_, e))| e.eq_ignore_ascii_case(commit_email))
                .map(|(i, _)| i)
                .collect()
        };

        let all_indices: Vec<usize> = canonical_indices
            .iter()
            .chain(commit_indices.iter())
            .copied()
            .collect();

        for w in all_indices.windows(2) {
            union(parent, rank, w[0], w[1]);
        }

        if let Some(ref name) = entry.canonical_name {
            if let Some(&first) = all_indices.first() {
                let root = find(parent, first);
                canonical_names.insert(root, name.clone());
            }
        }
    }

    canonical_names
}

fn find(parent: &mut [usize], i: usize) -> usize {
    if parent[i] != i {
        parent[i] = find(parent, parent[i]);
    }
    parent[i]
}

fn union(parent: &mut [usize], rank: &mut [usize], i: usize, j: usize) {
    let ri = find(parent, i);
    let rj = find(parent, j);
    if ri == rj {
        return;
    }
    if rank[ri] < rank[rj] {
        parent[ri] = rj;
    } else if rank[ri] > rank[rj] {
        parent[rj] = ri;
    } else {
        parent[rj] = ri;
        rank[ri] += 1;
    }
}
