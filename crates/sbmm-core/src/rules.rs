//! Load order rules: "this mod loads after that one".
//!
//! Users write them for mods that must override others whatever else
//! changes, as LOOT and Vortex let them. [`sort`] reorders the list to obey
//! every rule while moving as little as it can: among the mods free to go
//! next, the one earliest in the current order always goes first, so a list
//! that already obeys the rules comes back unchanged.

use std::collections::{BTreeMap, BTreeSet};

/// `before` must load before `after` — `after` wins where they clash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rule {
    pub before: i64,
    pub after: i64,
}

/// Rules the order breaks. Rules naming a mod not in the order are ignored.
pub fn broken(order: &[i64], rules: &[Rule]) -> Vec<Rule> {
    let position: BTreeMap<i64, usize> = order.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    rules
        .iter()
        .filter(
            |rule| match (position.get(&rule.before), position.get(&rule.after)) {
                (Some(before), Some(after)) => before > after,
                _ => false,
            },
        )
        .copied()
        .collect()
}

/// Whether adding `rule` would make the rules impossible to satisfy.
pub fn creates_cycle(rules: &[Rule], rule: Rule) -> bool {
    if rule.before == rule.after {
        return true;
    }
    // A cycle exists if `after` already has to come before `before`.
    let mut stack = vec![rule.after];
    let mut seen = BTreeSet::new();
    while let Some(node) = stack.pop() {
        if node == rule.before {
            return true;
        }
        if seen.insert(node) {
            stack.extend(rules.iter().filter(|r| r.before == node).map(|r| r.after));
        }
    }
    false
}

/// `order` rearranged to obey `rules`, or `None` if they contradict each
/// other.
pub fn sort(order: &[i64], rules: &[Rule]) -> Option<Vec<i64>> {
    let present: BTreeSet<i64> = order.iter().copied().collect();
    let position: BTreeMap<i64, usize> = order.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let rules: BTreeSet<Rule> = rules
        .iter()
        .filter(|r| present.contains(&r.before) && present.contains(&r.after))
        .copied()
        .collect();

    let mut waiting_on: BTreeMap<i64, usize> = order.iter().map(|id| (*id, 0)).collect();
    for rule in &rules {
        *waiting_on.get_mut(&rule.after).expect("present") += 1;
    }
    // Ready mods keyed by where they are now.
    let mut ready: BTreeSet<(usize, i64)> = waiting_on
        .iter()
        .filter(|(_, n)| **n == 0)
        .map(|(id, _)| (position[id], *id))
        .collect();

    let mut sorted = Vec::with_capacity(order.len());
    while let Some(next) = ready.pop_first() {
        let id = next.1;
        sorted.push(id);
        for rule in rules.iter().filter(|r| r.before == id) {
            let count = waiting_on.get_mut(&rule.after).expect("present");
            *count -= 1;
            if *count == 0 {
                ready.insert((position[&rule.after], rule.after));
            }
        }
    }
    (sorted.len() == order.len()).then_some(sorted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(before: i64, after: i64) -> Rule {
        Rule { before, after }
    }

    #[test]
    fn an_order_that_obeys_the_rules_is_left_alone() {
        let order = [1, 2, 3, 4];
        assert_eq!(sort(&order, &[rule(1, 3), rule(2, 4)]).unwrap(), order);
        assert!(broken(&order, &[rule(1, 3)]).is_empty());
    }

    #[test]
    fn a_broken_rule_moves_as_little_as_it_can() {
        // 4 must load before 2: it moves up just ahead of 2.
        let order = [1, 2, 3, 4, 5];
        assert_eq!(broken(&order, &[rule(4, 2)]), [rule(4, 2)]);
        assert_eq!(sort(&order, &[rule(4, 2)]).unwrap(), [1, 3, 4, 2, 5]);
    }

    #[test]
    fn rules_about_mods_not_in_the_list_are_ignored() {
        assert_eq!(sort(&[1, 2], &[rule(9, 1), rule(2, 7)]).unwrap(), [1, 2]);
    }

    #[test]
    fn contradicting_rules_are_refused() {
        let rules = [rule(1, 2), rule(2, 3)];
        assert!(creates_cycle(&rules, rule(3, 1)));
        assert!(creates_cycle(&rules, rule(2, 2)));
        assert!(!creates_cycle(&rules, rule(1, 3)));
        assert_eq!(
            sort(&[1, 2, 3], &[rule(1, 2), rule(2, 3), rule(3, 1)]),
            None
        );
    }
}
