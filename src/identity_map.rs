use crate::util::{load_or_default, write_atomic};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct IdentityMap {
    #[serde(default)]
    pub group: Vec<MapGroup>,
    #[serde(default)]
    pub reject: Vec<MapPair>,
    #[serde(default)]
    pub unsure: Vec<MapPair>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct MapGroup {
    pub name: String,
    pub members: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct MapPair {
    pub a: String,
    pub b: String,
}

pub fn format_identity(name: &str, email: &str) -> String {
    format!("{} <{}>", name, email)
}

pub fn parse_identity(s: &str) -> Option<(String, String)> {
    let s = s.trim();
    let bracket_start = s.rfind('<')?;
    let bracket_end = s.rfind('>')?;
    if bracket_end <= bracket_start {
        return None;
    }
    let name = s[..bracket_start].trim().to_string();
    let email = s[bracket_start + 1..bracket_end].trim().to_string();
    Some((name, email))
}

impl IdentityMap {
    pub fn load(path: &Path) -> Self {
        load_or_default(path, |s| toml::from_str::<Self>(s))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let mut out = String::from(
            "# Gild - Identity Map\n\
             #\n\
             # [[group]]:  confirmed same-person identities\n\
             # [[reject]]: confirmed different-person pairs (won't be asked again)\n\
             # [[unsure]]: marked don't-know (won't be asked again; promote to [[group]] or [[reject]] manually)\n\
             #\n\
             # Edit freely. Changes take effect on next run.\n\n",
        );
        out.push_str(&toml::to_string(self)?);
        write_atomic(path, out.as_bytes())
    }

    fn pair_matches(list: &[MapPair], a: &str, b: &str) -> bool {
        list.iter()
            .any(|p| (p.a == a && p.b == b) || (p.a == b && p.b == a))
    }

    pub fn is_rejected(&self, a: &str, b: &str) -> bool {
        Self::pair_matches(&self.reject, a, b)
    }

    pub fn is_unsure(&self, a: &str, b: &str) -> bool {
        Self::pair_matches(&self.unsure, a, b)
    }

    pub fn find_group_for_member(&self, identity: &str) -> Option<usize> {
        self.group
            .iter()
            .position(|g| g.members.iter().any(|m| m == identity))
    }

    pub fn add_merge(&mut self, group_a: &[(String, String)], group_b: &[(String, String)]) {
        let formatted_a: Vec<String> = group_a
            .iter()
            .map(|(n, e)| format_identity(n, e))
            .collect();
        let formatted_b: Vec<String> = group_b
            .iter()
            .map(|(n, e)| format_identity(n, e))
            .collect();

        let existing_idx_a = formatted_a
            .iter()
            .find_map(|f| self.find_group_for_member(f));
        let existing_idx_b = formatted_b
            .iter()
            .find_map(|f| self.find_group_for_member(f));

        match (existing_idx_a, existing_idx_b) {
            (Some(ia), Some(ib)) if ia != ib => {
                let members_b = self.group[ib].members.clone();
                for m in members_b {
                    if !self.group[ia].members.contains(&m) {
                        self.group[ia].members.push(m);
                    }
                }
                self.group.remove(ib);
            }
            (Some(ia), None) => {
                for f in &formatted_b {
                    if !self.group[ia].members.contains(f) {
                        self.group[ia].members.push(f.clone());
                    }
                }
            }
            (None, Some(ib)) => {
                for f in &formatted_a {
                    if !self.group[ib].members.contains(f) {
                        self.group[ib].members.push(f.clone());
                    }
                }
            }
            (None, None) => {
                let mut all_members = formatted_a;
                for f in formatted_b {
                    if !all_members.contains(&f) {
                        all_members.push(f);
                    }
                }
                let name = group_a
                    .iter()
                    .chain(group_b.iter())
                    .max_by_key(|(n, _)| n.len())
                    .map(|(n, _)| n.clone())
                    .unwrap_or_else(|| "Unknown".to_string());
                self.group.push(MapGroup {
                    name,
                    members: all_members,
                });
            }
            _ => {}
        }
    }

    pub fn add_reject(&mut self, a: &[(String, String)], b: &[(String, String)]) {
        let fa = format_identity(&a[0].0, &a[0].1);
        let fb = format_identity(&b[0].0, &b[0].1);
        if !self.is_rejected(&fa, &fb) {
            self.reject.push(MapPair { a: fa, b: fb });
        }
    }

    pub fn add_unsure(&mut self, a: &[(String, String)], b: &[(String, String)]) {
        let fa = format_identity(&a[0].0, &a[0].1);
        let fb = format_identity(&b[0].0, &b[0].1);
        if !self.is_unsure(&fa, &fb) {
            self.unsure.push(MapPair { a: fa, b: fb });
        }
    }
}
