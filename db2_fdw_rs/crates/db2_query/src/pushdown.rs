//! WHERE clause pushdown checking
//!
//! This module determines which expressions can be pushed down to DB2
//! for remote execution.

use thiserror::Error;

/// Errors during pushdown checking
#[derive(Error, Debug)]
pub enum PushdownError {
    #[error("Expression not pushable: {reason}")]
    NotPushable { reason: String },

    #[error("Unsupported operator: {0}")]
    UnsupportedOperator(String),

    #[error("Unsupported function: {0}")]
    UnsupportedFunction(String),
}

/// Capabilities for pushdown
#[derive(Debug, Clone, Default)]
pub struct PushdownCapability {
    /// Can push simple comparisons (=, <>, <, >, <=, >=)
    pub comparisons: bool,
    /// Can push LIKE patterns
    pub like: bool,
    /// Can push IN lists
    pub in_list: bool,
    /// Can push BETWEEN
    pub between: bool,
    /// Can push IS NULL / IS NOT NULL
    pub null_tests: bool,
    /// Can push AND/OR
    pub boolean_ops: bool,
    /// Can push NOT
    pub not: bool,
    /// Can push arithmetic operators
    pub arithmetic: bool,
    /// Can push aggregate functions
    pub aggregates: bool,
    /// Can push JOINs
    pub joins: bool,
    /// Can push ORDER BY
    pub order_by: bool,
    /// Can push LIMIT/OFFSET
    pub limit: bool,
}

impl PushdownCapability {
    /// Create capabilities for full pushdown
    pub fn full() -> Self {
        Self {
            comparisons: true,
            like: true,
            in_list: true,
            between: true,
            null_tests: true,
            boolean_ops: true,
            not: true,
            arithmetic: true,
            aggregates: true,
            joins: true,
            order_by: true,
            limit: true,
        }
    }

    /// Create capabilities for no pushdown
    pub fn none() -> Self {
        Self::default()
    }

    /// Create default DB2 FDW capabilities
    pub fn db2_default() -> Self {
        Self {
            comparisons: true,
            like: true,
            in_list: true,
            between: true,
            null_tests: true,
            boolean_ops: true,
            not: true,
            arithmetic: true,
            aggregates: false,  // Conservative default
            joins: true,
            order_by: true,
            limit: true,
        }
    }
}

/// Checker for pushdown eligibility
pub struct PushdownChecker {
    capabilities: PushdownCapability,
}

impl PushdownChecker {
    /// Create a new checker with specified capabilities
    pub fn new(capabilities: PushdownCapability) -> Self {
        Self { capabilities }
    }

    /// Create a checker with default DB2 capabilities
    pub fn db2_default() -> Self {
        Self::new(PushdownCapability::db2_default())
    }

    /// Check if a comparison operator can be pushed down
    pub fn can_push_comparison(&self, op: &str) -> bool {
        if !self.capabilities.comparisons {
            return false;
        }

        matches!(op, "=" | "<>" | "!=" | "<" | ">" | "<=" | ">=")
    }

    /// Check if LIKE can be pushed down
    pub fn can_push_like(&self) -> bool {
        self.capabilities.like
    }

    /// Check if IN list can be pushed down
    pub fn can_push_in(&self, list_size: usize) -> bool {
        if !self.capabilities.in_list {
            return false;
        }
        // DB2 has a limit on IN list size
        list_size <= 1000
    }

    /// Check if BETWEEN can be pushed down
    pub fn can_push_between(&self) -> bool {
        self.capabilities.between
    }

    /// Check if NULL test can be pushed down
    pub fn can_push_null_test(&self) -> bool {
        self.capabilities.null_tests
    }

    /// Check if a function can be pushed down
    pub fn can_push_function(&self, func_name: &str) -> bool {
        // List of functions that DB2 supports and we can safely push down
        let safe_functions = [
            // String functions
            "upper", "lower", "length", "substr", "substring", "trim",
            "ltrim", "rtrim", "concat", "replace", "position", "locate",
            // Numeric functions
            "abs", "ceil", "ceiling", "floor", "round", "mod", "power",
            "sqrt", "sign", "trunc", "truncate",
            // Date/time functions
            "current_date", "current_time", "current_timestamp",
            "year", "month", "day", "hour", "minute", "second",
            "date", "time", "timestamp",
            // Other
            "coalesce", "nullif", "cast",
        ];

        safe_functions.contains(&func_name.to_lowercase().as_str())
    }

    /// Check if an aggregate can be pushed down
    pub fn can_push_aggregate(&self, agg_name: &str) -> bool {
        if !self.capabilities.aggregates {
            return false;
        }

        let safe_aggregates = [
            "count", "sum", "avg", "min", "max",
            "stddev", "variance",
        ];

        safe_aggregates.contains(&agg_name.to_lowercase().as_str())
    }

    /// Check if ORDER BY can be pushed down
    pub fn can_push_order_by(&self) -> bool {
        self.capabilities.order_by
    }

    /// Check if LIMIT can be pushed down
    pub fn can_push_limit(&self) -> bool {
        self.capabilities.limit
    }

    /// Check if a JOIN can be pushed down
    pub fn can_push_join(&self, join_type: JoinType) -> bool {
        if !self.capabilities.joins {
            return false;
        }

        // DB2 supports all standard join types
        matches!(
            join_type,
            JoinType::Inner | JoinType::Left | JoinType::Right | JoinType::Full
        )
    }
}

/// Types of joins
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinType {
    Inner,
    Left,
    Right,
    Full,
    Cross,
}

impl JoinType {
    pub fn to_sql(&self) -> &'static str {
        match self {
            JoinType::Inner => "INNER JOIN",
            JoinType::Left => "LEFT OUTER JOIN",
            JoinType::Right => "RIGHT OUTER JOIN",
            JoinType::Full => "FULL OUTER JOIN",
            JoinType::Cross => "CROSS JOIN",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_comparison_pushdown() {
        let checker = PushdownChecker::db2_default();

        assert!(checker.can_push_comparison("="));
        assert!(checker.can_push_comparison("<>"));
        assert!(checker.can_push_comparison("<"));
        assert!(!checker.can_push_comparison("~"));
    }

    #[test]
    fn test_function_pushdown() {
        let checker = PushdownChecker::db2_default();

        assert!(checker.can_push_function("UPPER"));
        assert!(checker.can_push_function("lower"));
        assert!(checker.can_push_function("COALESCE"));
        assert!(!checker.can_push_function("pg_specific_func"));
    }

    #[test]
    fn test_in_list_limit() {
        let checker = PushdownChecker::db2_default();

        assert!(checker.can_push_in(10));
        assert!(checker.can_push_in(1000));
        assert!(!checker.can_push_in(1001));
    }

    #[test]
    fn test_join_types() {
        let checker = PushdownChecker::db2_default();

        assert!(checker.can_push_join(JoinType::Inner));
        assert!(checker.can_push_join(JoinType::Left));
        assert!(checker.can_push_join(JoinType::Full));
    }
}
