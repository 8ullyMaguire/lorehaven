//! M15 — Fair queue domain: position assignment per priority class.

/// Priority classes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PriorityClass {
    Free,
    Priority,
    Subscription,
}

impl PriorityClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Priority => "priority",
            Self::Subscription => "subscription",
        }
    }
}

/// Compute the next position for a job within its priority class.
/// Positions are 1-indexed and monotonic within a class.
pub fn next_position(current_max: Option<i64>) -> i64 {
    current_max.map(|m| m + 1).unwrap_or(1)
}

/// Queue ordering key: priority class first, then position within class.
/// Lower values = higher priority.
pub fn queue_key(class: &PriorityClass, position: i64) -> (i32, i64) {
    let class_order = match class {
        PriorityClass::Subscription => 0,
        PriorityClass::Priority => 1,
        PriorityClass::Free => 2,
    };
    (class_order, position)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_position_is_one() {
        assert_eq!(next_position(None), 1);
    }

    #[test]
    fn positions_are_monotonic() {
        assert_eq!(next_position(Some(5)), 6);
    }

    #[test]
    fn subscription_ranks_above_free() {
        let sub = queue_key(&PriorityClass::Subscription, 5);
        let free = queue_key(&PriorityClass::Free, 1);
        assert!(sub < free);
    }

    #[test]
    fn priority_ranks_above_free() {
        let pri = queue_key(&PriorityClass::Priority, 5);
        let free = queue_key(&PriorityClass::Free, 1);
        assert!(pri < free);
    }
}
