//! PostgreSQL Expression Deparsing
//!
//! This module extracts WHERE clauses from PostgreSQL plan nodes
//! and converts them to DB2 SQL for predicate pushdown.

use pgrx::pg_sys;
use tracing::{debug, warn};

use db2_query::pushdown::PushdownChecker;

// Use safe FFI wrappers
use crate::safe_ffi;

/// Result of checking if a clause can be pushed down
pub struct PushdownResult {
    /// SQL expression that can be pushed to DB2
    pub remote_conds: Vec<String>,
    /// Expressions that must be evaluated locally
    pub local_conds: Vec<String>,
}

impl PushdownResult {
    pub fn new() -> Self {
        Self {
            remote_conds: Vec::new(),
            local_conds: Vec::new(),
        }
    }

    /// Get the WHERE clause for remote execution
    pub fn remote_where(&self) -> Option<String> {
        if self.remote_conds.is_empty() {
            None
        } else {
            Some(self.remote_conds.join(" AND "))
        }
    }
}

/// Extract and classify WHERE clauses for pushdown
///
/// Analyzes PostgreSQL RestrictInfo nodes and determines which
/// can be pushed to DB2 for remote execution.
pub unsafe fn classify_conditions(
    baserel: *mut pg_sys::RelOptInfo,
    checker: &PushdownChecker,
) -> PushdownResult {
    let mut result = PushdownResult::new();

    if baserel.is_null() {
        return result;
    }

    // Get the baserestrictinfo list
    let restrict_list = (*baserel).baserestrictinfo;
    if restrict_list.is_null() {
        return result;
    }

    let list_len = (*restrict_list).length;
    debug!("Classifying {} restriction clauses", list_len);

    for i in 0..list_len {
        let cell = pg_sys::list_nth_cell(restrict_list, i);
        if cell.is_null() {
            continue;
        }

        let rinfo = (*cell).ptr_value as *mut pg_sys::RestrictInfo;
        if rinfo.is_null() {
            continue;
        }

        // Get the clause expression
        let clause = (*rinfo).clause;
        if clause.is_null() {
            continue;
        }

        // Try to deparse the expression
        match deparse_expr(clause, checker) {
            Some(sql) => {
                debug!("Pushable clause: {}", sql);
                result.remote_conds.push(sql);
            }
            None => {
                debug!("Non-pushable clause, will evaluate locally");
                result.local_conds.push("(local)".to_string());
            }
        }
    }

    result
}

/// Deparse a PostgreSQL expression to DB2 SQL
///
/// Returns None if the expression cannot be pushed down.
unsafe fn deparse_expr(
    expr: *mut pg_sys::Expr,
    checker: &PushdownChecker,
) -> Option<String> {
    if expr.is_null() {
        return None;
    }

    let node_tag = (*expr).type_;

    match node_tag {
        // OpExpr: binary operators like =, <, >, etc.
        pg_sys::NodeTag::T_OpExpr => {
            deparse_op_expr(expr as *mut pg_sys::OpExpr, checker)
        }

        // Var: column reference
        pg_sys::NodeTag::T_Var => {
            deparse_var(expr as *mut pg_sys::Var)
        }

        // Const: literal value
        pg_sys::NodeTag::T_Const => {
            deparse_const(expr as *mut pg_sys::Const)
        }

        // BoolExpr: AND, OR, NOT
        pg_sys::NodeTag::T_BoolExpr => {
            deparse_bool_expr(expr as *mut pg_sys::BoolExpr, checker)
        }

        // NullTest: IS NULL, IS NOT NULL
        pg_sys::NodeTag::T_NullTest => {
            deparse_null_test(expr as *mut pg_sys::NullTest, checker)
        }

        // ScalarArrayOpExpr: IN (list)
        pg_sys::NodeTag::T_ScalarArrayOpExpr => {
            deparse_scalar_array_op(expr as *mut pg_sys::ScalarArrayOpExpr, checker)
        }

        _ => {
            debug!("Unsupported node type for pushdown: {:?}", node_tag);
            None
        }
    }
}

/// Deparse an operator expression (e.g., col = 5, col > 10)
unsafe fn deparse_op_expr(
    op_expr: *mut pg_sys::OpExpr,
    checker: &PushdownChecker,
) -> Option<String> {
    if op_expr.is_null() {
        return None;
    }

    // Get operator name
    let opno = (*op_expr).opno;
    let op_name = get_operator_name(opno)?;

    // Check if operator can be pushed down
    if !checker.can_push_comparison(&op_name) {
        return None;
    }

    // Get operands
    let args = (*op_expr).args;
    if args.is_null() || (*args).length != 2 {
        return None;
    }

    let left_cell = pg_sys::list_nth_cell(args, 0);
    let right_cell = pg_sys::list_nth_cell(args, 1);

    if left_cell.is_null() || right_cell.is_null() {
        return None;
    }

    let left = (*left_cell).ptr_value as *mut pg_sys::Expr;
    let right = (*right_cell).ptr_value as *mut pg_sys::Expr;

    let left_sql = deparse_expr(left, checker)?;
    let right_sql = deparse_expr(right, checker)?;

    Some(format!("{} {} {}", left_sql, op_name, right_sql))
}

/// Deparse a Var (column reference)
unsafe fn deparse_var(var: *mut pg_sys::Var) -> Option<String> {
    if var.is_null() {
        return None;
    }

    // Get the attribute number
    let attno = (*var).varattno;

    if attno <= 0 {
        // System column, can't push down
        return None;
    }

    // For now, just return a placeholder - in real implementation,
    // we'd look up the column name from the relation
    Some(format!("col{}", attno))
}

/// Deparse a Const (literal value)
unsafe fn deparse_const(const_expr: *mut pg_sys::Const) -> Option<String> {
    if const_expr.is_null() {
        return None;
    }

    if (*const_expr).constisnull {
        return Some("NULL".to_string());
    }

    let typid = (*const_expr).consttype;
    let datum = (*const_expr).constvalue;

    // Convert based on type
    match typid {
        pg_sys::INT2OID => Some(format!("{}", datum.value() as i16)),
        pg_sys::INT4OID => Some(format!("{}", datum.value() as i32)),
        pg_sys::INT8OID => Some(format!("{}", datum.value() as i64)),
        pg_sys::FLOAT4OID => Some(format!("{}", f32::from_bits(datum.value() as u32))),
        pg_sys::FLOAT8OID => Some(format!("{}", f64::from_bits(datum.value() as u64))),
        pg_sys::BOOLOID => Some(if datum.value() != 0 { "TRUE" } else { "FALSE" }.to_string()),
        pg_sys::TEXTOID | pg_sys::VARCHAROID | pg_sys::BPCHAROID => {
            // Use safe wrapper with validation
            unsafe {
                match safe_ffi::datum_get_text(typid, datum) {
                    Ok(s) => {
                        // Escape single quotes
                        Some(format!("'{}'", s.replace('\'', "''")))
                    },
                    Err(e) => {
                        warn!("Failed to extract text datum: {}", e);
                        None // Cannot push down invalid data
                    }
                }
            }
        }
        _ => {
            // Try to use output function with safe wrapper
            unsafe {
                match safe_ffi::get_type_output_info(typid) {
                    Ok(output_func) => {
                        match safe_ffi::oid_output_call(output_func, datum) {
                            Ok(s) => {
                                // Escape single quotes
                                Some(format!("'{}'", s.replace('\'', "''")))
                            },
                            Err(e) => {
                                warn!("Failed to call output function: {}", e);
                                None
                            }
                        }
                    },
                    Err(e) => {
                        warn!("Failed to get output function for type {:?}: {}", typid, e);
                        None
                    }
                }
            }
        }
    }
}

/// Deparse a BoolExpr (AND, OR, NOT)
unsafe fn deparse_bool_expr(
    bool_expr: *mut pg_sys::BoolExpr,
    checker: &PushdownChecker,
) -> Option<String> {
    if bool_expr.is_null() {
        return None;
    }

    let bool_type = (*bool_expr).boolop;
    let args = (*bool_expr).args;

    if args.is_null() {
        return None;
    }

    let list_len = (*args).length;

    match bool_type {
        pg_sys::BoolExprType::AND_EXPR => {
            let mut parts = Vec::new();
            for i in 0..list_len {
                let cell = pg_sys::list_nth_cell(args, i);
                if cell.is_null() {
                    continue;
                }
                let arg = (*cell).ptr_value as *mut pg_sys::Expr;
                if let Some(sql) = deparse_expr(arg, checker) {
                    parts.push(sql);
                } else {
                    // If any part can't be pushed, the whole AND can't be pushed
                    return None;
                }
            }
            if parts.is_empty() {
                None
            } else {
                Some(format!("({})", parts.join(" AND ")))
            }
        }
        pg_sys::BoolExprType::OR_EXPR => {
            let mut parts = Vec::new();
            for i in 0..list_len {
                let cell = pg_sys::list_nth_cell(args, i);
                if cell.is_null() {
                    continue;
                }
                let arg = (*cell).ptr_value as *mut pg_sys::Expr;
                if let Some(sql) = deparse_expr(arg, checker) {
                    parts.push(sql);
                } else {
                    // If any part can't be pushed, the whole OR can't be pushed
                    return None;
                }
            }
            if parts.is_empty() {
                None
            } else {
                Some(format!("({})", parts.join(" OR ")))
            }
        }
        pg_sys::BoolExprType::NOT_EXPR => {
            if list_len != 1 {
                return None;
            }
            let cell = pg_sys::list_nth_cell(args, 0);
            if cell.is_null() {
                return None;
            }
            let arg = (*cell).ptr_value as *mut pg_sys::Expr;
            deparse_expr(arg, checker).map(|sql| format!("NOT ({})", sql))
        }
        _ => None, // Unsupported bool expression types for pushdown
    }
}

/// Deparse a NullTest (IS NULL, IS NOT NULL)
unsafe fn deparse_null_test(
    null_test: *mut pg_sys::NullTest,
    checker: &PushdownChecker,
) -> Option<String> {
    if null_test.is_null() || !checker.can_push_null_test() {
        return None;
    }

    let arg = (*null_test).arg as *mut pg_sys::Expr;
    let arg_sql = deparse_expr(arg, checker)?;

    let op = match (*null_test).nulltesttype {
        pg_sys::NullTestType::IS_NULL => "IS NULL",
        pg_sys::NullTestType::IS_NOT_NULL => "IS NOT NULL",
        _ => "IS NULL", // Default for any new types added in future PG versions
    };

    Some(format!("{} {}", arg_sql, op))
}

/// Deparse a ScalarArrayOpExpr (IN list)
unsafe fn deparse_scalar_array_op(
    op_expr: *mut pg_sys::ScalarArrayOpExpr,
    checker: &PushdownChecker,
) -> Option<String> {
    if op_expr.is_null() {
        return None;
    }

    // Get the arguments
    let args = (*op_expr).args;
    if args.is_null() || (*args).length != 2 {
        return None;
    }

    let left_cell = pg_sys::list_nth_cell(args, 0);
    let right_cell = pg_sys::list_nth_cell(args, 1);

    if left_cell.is_null() || right_cell.is_null() {
        return None;
    }

    let left = (*left_cell).ptr_value as *mut pg_sys::Expr;
    let left_sql = deparse_expr(left, checker)?;

    // Right side should be an ArrayExpr or Const array
    // For simplicity, we'll handle const arrays
    let right = (*right_cell).ptr_value as *mut pg_sys::Expr;

    if (*right).type_ == pg_sys::NodeTag::T_Const {
        // Array constant - would need to expand
        // For now, skip complex array handling
        warn!("Array constant IN lists not yet fully supported");
        return None;
    }

    // Check if it's an ArrayExpr
    if (*right).type_ == pg_sys::NodeTag::T_ArrayExpr {
        let arr_expr = right as *mut pg_sys::ArrayExpr;
        let elements = (*arr_expr).elements;

        if elements.is_null() {
            return None;
        }

        let list_len = (*elements).length as usize;
        if !checker.can_push_in(list_len) {
            return None;
        }

        let mut values = Vec::new();
        for i in 0..(list_len as i32) {
            let cell = pg_sys::list_nth_cell(elements, i);
            if cell.is_null() {
                continue;
            }
            let elem = (*cell).ptr_value as *mut pg_sys::Expr;
            if let Some(sql) = deparse_expr(elem, checker) {
                values.push(sql);
            } else {
                return None;
            }
        }

        let op = if (*op_expr).useOr { "IN" } else { "NOT IN" };
        return Some(format!("{} {} ({})", left_sql, op, values.join(", ")));
    }

    None
}

/// Get operator name from OID
unsafe fn get_operator_name(opno: pg_sys::Oid) -> Option<String> {
    // Cannot match against Oid(t) pattern because Oid is a struct with private fields
    // Instead, extract the numeric value using Debug formatting
    let oid_num = format!("{:?}", opno);

    // Parse numeric value from Oid debug output (format is "Oid(123)")
    let oid_num_str = oid_num.strip_prefix("Oid(").unwrap_or("0")
        .strip_suffix(")").unwrap_or("0");

    match oid_num_str.parse::<u32>() {
        Ok(96) => Some("=".to_string()),    // int4eq
        Ok(97) => Some("<".to_string()),    // int4lt
        Ok(521) => Some(">".to_string()),   // int4gt
        Ok(523) => Some("<=".to_string()),  // int4le
        Ok(525) => Some(">=".to_string()),  // int4ge
        Ok(518) => Some("<>".to_string()),  // int4ne

        // Text operators
        Ok(98) => Some("=".to_string()),    // texteq
        Ok(664) => Some("<".to_string()),   // text_lt
        Ok(666) => Some("<=".to_string()),  // text_le
        Ok(665) => Some(">".to_string()),   // text_gt
        Ok(667) => Some(">=".to_string()),  // text_ge
        Ok(531) => Some("<>".to_string()),  // textne

        // Not in common list, try catalog lookup
        _ => {
            // Try to get from catalog
            let opr = pg_sys::SearchSysCache1(
                pg_sys::SysCacheIdentifier::OPEROID as i32,
                pg_sys::Datum::from(opno),
            );
            if opr.is_null() {
                return None;
            }

            let form = pg_sys::GETSTRUCT(opr) as *mut pg_sys::FormData_pg_operator;
            let oprname = std::ffi::CStr::from_ptr((*form).oprname.data.as_ptr())
                .to_string_lossy()
                .to_string();

            pg_sys::ReleaseSysCache(opr);
            Some(oprname)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pushdown_result() {
        let mut result = PushdownResult::new();
        result.remote_conds.push("col1 = 5".to_string());
        result.remote_conds.push("col2 > 10".to_string());

        assert_eq!(result.remote_where(), Some("col1 = 5 AND col2 > 10".to_string()));
    }
}
