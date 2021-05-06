#![no_main]
use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use regalloc2::set::IntSet;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq, Arbitrary)]
enum Action {
    AddLeft(u16),
    AddRight(u16),
    RemoveLeft(u16),
    RemoveRight(u16),
    LeftContains(u16),
    RightContains(u16),
    ClearLeft,
    ClearRight,
    MergeLeftToRight,
    MergeRightToLeft,
}

#[derive(Debug, Arbitrary)]
struct TestCase {
    actions: Vec<Action>,
}

fn remove_dups(list: &mut Vec<usize>) {
    let mut out_idx = 0;
    let mut last = None;
    for i in 0..list.len() {
        if Some(list[i]) != last {
            if out_idx < i {
                list[out_idx] = list[i];
            }
            out_idx += 1;
        }
        last = Some(list[i]);
    }
    list.truncate(out_idx);
}

fn assert_set_eq(oracle: &HashSet<usize>, test: &IntSet) {
    let mut a: Vec<usize> = oracle.iter().cloned().collect();
    let mut b: Vec<usize> = test.iter().collect();
    a.sort();
    b.sort();
    remove_dups(&mut b);
    assert_eq!(a, b);
}

fuzz_target!(|testcase: TestCase| {
    let mut left_oracle = HashSet::new();
    let mut right_oracle = HashSet::new();
    let mut left = IntSet::new();
    let mut right = IntSet::new();

    for action in &testcase.actions {
        assert_set_eq(&left_oracle, &left);
        assert_set_eq(&right_oracle, &right);
        match action {
            &Action::AddLeft(val) => {
                let val = val as usize;
                left_oracle.insert(val);
                left.add(val);
            }
            &Action::AddRight(val) => {
                let val = val as usize;
                right_oracle.insert(val);
                right.add(val);
            }
            &Action::RemoveLeft(val) => {
                let val = val as usize;
                let x = left_oracle.remove(&val);
                let y = left.contains(val);
                assert_eq!(x, y);
                left.remove(val);
            }
            &Action::RemoveRight(val) => {
                let val = val as usize;
                let x = right_oracle.remove(&val);
                let y = right.contains(val);
                assert_eq!(x, y);
                right.remove(val);
            }
            &Action::LeftContains(val) => {
                let val = val as usize;
                let x = left_oracle.contains(&val);
                let y = left.contains(val);
                assert_eq!(x, y);
            }
            &Action::RightContains(val) => {
                let val = val as usize;
                let x = right_oracle.contains(&val);
                let y = right.contains(val);
                assert_eq!(x, y);
            }
            &Action::ClearLeft => {
                left_oracle.clear();
                left.clear();
            }
            &Action::ClearRight => {
                right_oracle.clear();
                right.clear();
            }
            &Action::MergeLeftToRight => {
                let before = right_oracle.clone();
                for &val in &left_oracle {
                    right_oracle.insert(val);
                }
                let x = right_oracle != before;
                let y = right.merge(&mut left);
                assert_eq!(x, y);
            }
            &Action::MergeRightToLeft => {
                let before = left_oracle.clone();
                for &val in &right_oracle {
                    left_oracle.insert(val);
                }
                let x = left_oracle != before;
                let y = left.merge(&mut right);
                assert_eq!(x, y);
            }
        }
    }
});
