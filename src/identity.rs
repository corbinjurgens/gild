use crate::git::Commit;
use crate::identity_map::{parse_identity, IdentityMap};
use crate::mailmap::MailmapEntry;
use std::collections::HashMap;

pub struct IdentityGroup {
    pub display_name: String,
    pub aliases: Vec<(String, String)>,
}

pub fn merge<'a>(
    commits: &'a [Commit],
    identity_map: &IdentityMap,
    mailmap: &[MailmapEntry],
) -> (Vec<IdentityGroup>, Vec<usize>) {
    let mut pair_counts: HashMap<(&'a str, &'a str), usize> = HashMap::new();
    for c in commits {
        *pair_counts
            .entry((c.author_name.as_str(), c.author_email.as_str()))
            .or_insert(0) += 1;
    }

    let pairs: Vec<(&'a str, &'a str)> = pair_counts.keys().copied().collect();
    let n = pairs.len();

    let pair_index: HashMap<(&str, &str), usize> =
        pairs.iter().enumerate().map(|(i, &p)| (p, i)).collect();

    let mut parent: Vec<usize> = (0..n).collect();
    let mut rank: Vec<usize> = vec![0; n];

    for group in &identity_map.group {
        let indices: Vec<usize> = group
            .members
            .iter()
            .filter_map(|m| parse_identity(m))
            .filter_map(|(n, e)| pair_index.get(&(n.as_str(), e.as_str())).copied())
            .collect();
        union_all(&mut parent, &mut rank, &indices);
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
        union_all(&mut parent, &mut rank, indices);
    }

    let mut group_members: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        group_members
            .entry(find(&mut parent, i))
            .or_default()
            .push(i);
    }

    let mut groups: Vec<IdentityGroup> = Vec::new();
    let mut pair_to_group: Vec<usize> = vec![usize::MAX; n];

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
                *name_counts.entry(pairs[m].0).or_insert(0) += count;
            }
            name_counts
                .iter()
                .max_by_key(|(name, count)| (*count, *name))
                .map(|(name, _)| name.to_string())
                .unwrap_or_else(|| "Unknown".to_string())
        };

        let aliases: Vec<(String, String)> = members
            .iter()
            .map(|&m| (pairs[m].0.to_string(), pairs[m].1.to_string()))
            .collect();

        groups.push(IdentityGroup {
            display_name,
            aliases,
        });

        for &m in members {
            pair_to_group[m] = group_idx;
        }
    }

    let assignments: Vec<usize> = commits
        .iter()
        .map(|c| {
            let pi = pair_index[&(c.author_name.as_str(), c.author_email.as_str())];
            pair_to_group[pi]
        })
        .collect();

    (groups, assignments)
}

fn union_all(parent: &mut [usize], rank: &mut [usize], indices: &[usize]) {
    for w in indices.windows(2) {
        union(parent, rank, w[0], w[1]);
    }
}

fn apply_mailmap(
    pairs: &[(&str, &str)],
    mailmap: &[MailmapEntry],
    parent: &mut [usize],
    rank: &mut [usize],
) -> HashMap<usize, String> {
    let mut canonical_names: HashMap<usize, String> = HashMap::new();

    let find_indices = |predicate: &dyn Fn(&str, &str) -> bool| -> Vec<usize> {
        pairs
            .iter()
            .enumerate()
            .filter(|(_, (n, e))| predicate(n, e))
            .map(|(i, _)| i)
            .collect()
    };

    for entry in mailmap {
        let commit_email = match &entry.commit_email {
            Some(e) => e,
            None => continue,
        };

        let canonical_indices =
            find_indices(&|_n, e| e.eq_ignore_ascii_case(&entry.canonical_email));

        let commit_indices = match &entry.commit_name {
            Some(cname) => find_indices(&|n, e| {
                e.eq_ignore_ascii_case(commit_email) && n.eq_ignore_ascii_case(cname)
            }),
            None => find_indices(&|_n, e| e.eq_ignore_ascii_case(commit_email)),
        };

        let all_indices: Vec<usize> = canonical_indices
            .iter()
            .chain(commit_indices.iter())
            .copied()
            .collect();

        union_all(parent, rank, &all_indices);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity_map::{format_identity, MapGroup};

    fn make_commit(name: &str, email: &str) -> Commit {
        Commit {
            author_name: name.into(),
            author_email: email.into(),
            group_id: 0,
            lines_added: 0,
            lines_removed: 0,
            files_changed: 0,
            timestamp: 0,
            whitespace_added: 0,
            whitespace_removed: 0,
            files_added: 0,
            files_deleted: 0,
            files_renamed: 0,
            is_merge: false,
        }
    }

    #[test]
    fn merge_empty() {
        let (groups, assignments) = merge(&[], &IdentityMap::default(), &[]);
        assert!(groups.is_empty());
        assert!(assignments.is_empty());
    }

    #[test]
    fn merge_single_author() {
        let commits = vec![make_commit("Alice", "alice@x.com")];
        let (groups, assignments) = merge(&commits, &IdentityMap::default(), &[]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].display_name, "Alice");
        assert_eq!(assignments, vec![0]);
    }

    #[test]
    fn merge_same_author_twice() {
        let commits = vec![
            make_commit("Alice", "alice@x.com"),
            make_commit("Alice", "alice@x.com"),
        ];
        let (groups, assignments) = merge(&commits, &IdentityMap::default(), &[]);
        assert_eq!(groups.len(), 1);
        assert_eq!(assignments[0], assignments[1]);
    }

    #[test]
    fn merge_auto_merge_same_email() {
        let commits = vec![
            make_commit("Alice Smith", "alice@x.com"),
            make_commit("alice", "alice@x.com"),
        ];
        let (groups, assignments) = merge(&commits, &IdentityMap::default(), &[]);
        assert_eq!(groups.len(), 1);
        assert_eq!(assignments[0], assignments[1]);
    }

    #[test]
    fn merge_distinct_authors() {
        let commits = vec![
            make_commit("Alice", "alice@x.com"),
            make_commit("Bob", "bob@x.com"),
        ];
        let (groups, assignments) = merge(&commits, &IdentityMap::default(), &[]);
        assert_eq!(groups.len(), 2);
        assert_ne!(assignments[0], assignments[1]);
    }

    #[test]
    fn merge_identity_map_merge() {
        let commits = vec![
            make_commit("Alice", "alice@work.com"),
            make_commit("Alice", "alice@home.com"),
        ];
        let identity_map = IdentityMap {
            group: vec![MapGroup {
                name: "Alice".into(),
                members: vec![
                    format_identity("Alice", "alice@work.com"),
                    format_identity("Alice", "alice@home.com"),
                ],
            }],
            ..IdentityMap::default()
        };
        let (groups, assignments) = merge(&commits, &identity_map, &[]);
        assert_eq!(groups.len(), 1);
        assert_eq!(assignments[0], assignments[1]);
    }

    #[test]
    fn merge_mailmap() {
        let commits = vec![
            make_commit("Old Name", "old@x.com"),
            make_commit("New Name", "new@x.com"),
        ];
        let mailmap = vec![MailmapEntry {
            canonical_name: Some("Canonical".into()),
            canonical_email: "new@x.com".into(),
            commit_name: None,
            commit_email: Some("old@x.com".into()),
        }];
        let (groups, assignments) = merge(&commits, &IdentityMap::default(), &mailmap);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].display_name, "Canonical");
        assert_eq!(assignments[0], assignments[1]);
    }

    #[test]
    fn merge_assignment_validity() {
        let commits = vec![
            make_commit("Alice", "a@x.com"),
            make_commit("Bob", "b@x.com"),
            make_commit("Alice", "a@x.com"),
        ];
        let (groups, assignments) = merge(&commits, &IdentityMap::default(), &[]);
        assert_eq!(assignments.len(), commits.len());
        assert!(assignments.iter().all(|&i| i < groups.len()));
    }
}
