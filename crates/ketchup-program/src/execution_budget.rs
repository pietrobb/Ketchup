//! Bounds interpreted work, including loops inside the frozen standard library.
use starlark::codemap::FileSpanRef;
use starlark::eval::{BeforeStmtFuncDyn, Evaluator};
use std::time::{Duration, Instant};

const MAX_STATEMENTS: u64 = 5_000_000;
const MAX_TIME: Duration = Duration::from_secs(20);

struct ExecutionBudget {
    statements: u64,
    started: Instant,
}

#[derive(Debug)]
struct ExecutionLimit;

impl std::fmt::Display for ExecutionLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("program execution budget exceeded; shorten loops or split the calculation")
    }
}

impl std::error::Error for ExecutionLimit {}

impl<'a, 'e: 'a> BeforeStmtFuncDyn<'a, 'e> for ExecutionBudget {
    fn call<'v>(
        &mut self,
        _span: FileSpanRef,
        _eval: &mut Evaluator<'v, 'a, 'e>,
    ) -> starlark::Result<()> {
        self.statements += 1;
        if self.statements > MAX_STATEMENTS
            || (self.statements.is_multiple_of(1024) && self.started.elapsed() > MAX_TIME)
        {
            return Err(starlark::Error::new_other(ExecutionLimit));
        }
        Ok(())
    }
}

pub(crate) fn install(eval: &mut Evaluator<'_, '_, '_>) {
    // Starlark 0.13 exposes the fallible statement hook through its DAP API.
    let hook: Box<dyn BeforeStmtFuncDyn<'_, '_>> = Box::new(ExecutionBudget {
        statements: 0,
        started: Instant::now(),
    });
    eval.before_stmt_for_dap(hook.into());
}
