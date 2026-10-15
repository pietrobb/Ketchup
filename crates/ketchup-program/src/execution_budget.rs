//! Bounds interpreted work, including loops inside the frozen standard library.
use starlark::codemap::FileSpanRef;
use starlark::eval::{BeforeStmtFuncDyn, Evaluator};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const MAX_STATEMENTS: u64 = 5_000_000;
/// Wall time one evaluation may take.
pub const MAX_TIME: Duration = Duration::from_secs(20);
/// Bytes the program heap may hold. Evaluation runs inside the window process,
/// so a program that builds a huge string or list must fail instead of taking
/// the memory of the whole computer.
pub const MAX_HEAP_BYTES: usize = 1 << 30;
/// Statements between two looks at the clock and the cancel flag.
const CHECK_EVERY: u64 = 256;

/// Stops an evaluation running on another thread: the program ends at its
/// next statement check with the error code `cancelled`. Clones share one flag.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// The shared flag, for code that polls it while waiting on another process.
    #[must_use]
    pub fn flag(&self) -> &AtomicBool {
        &self.0
    }
}

/// A token on a flag the caller already shares, such as a request's cancel flag.
impl From<Arc<AtomicBool>> for CancelToken {
    fn from(flag: Arc<AtomicBool>) -> Self {
        Self(flag)
    }
}

struct ExecutionBudget {
    statements: u64,
    max_statements: u64,
    started: Instant,
    cancel: CancelToken,
}

#[derive(Debug)]
struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("program evaluation cancelled: a newer program replaced it")
    }
}

impl std::error::Error for Cancelled {}

#[derive(Debug)]
struct ExecutionLimit;

impl std::fmt::Display for ExecutionLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("program execution budget exceeded; shorten loops or split the calculation")
    }
}

impl std::error::Error for ExecutionLimit {}

#[derive(Debug)]
struct MemoryLimit;

impl std::fmt::Display for MemoryLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("program memory limit exceeded; build smaller strings or lists")
    }
}

impl std::error::Error for MemoryLimit {}

impl<'a, 'e: 'a> BeforeStmtFuncDyn<'a, 'e> for ExecutionBudget {
    fn call<'v>(
        &mut self,
        _span: FileSpanRef,
        eval: &mut Evaluator<'v, 'a, 'e>,
    ) -> starlark::Result<()> {
        self.statements += 1;
        if self.statements.is_multiple_of(CHECK_EVERY) && self.cancel.is_cancelled() {
            return Err(starlark::Error::new_other(Cancelled));
        }
        // Every statement: one statement can double a string (`s = s + s`).
        if eval
            .heap()
            .allocation_limit()
            .is_some_and(|limit| eval.heap().allocated_bytes() > limit)
        {
            return Err(starlark::Error::new_other(MemoryLimit));
        }
        if self.statements > self.max_statements
            || (self.statements.is_multiple_of(CHECK_EVERY) && self.started.elapsed() > MAX_TIME)
        {
            return Err(starlark::Error::new_other(ExecutionLimit));
        }
        Ok(())
    }
}

pub(crate) fn install(eval: &mut Evaluator<'_, '_, '_>, cancel: &CancelToken) {
    install_with(eval, MAX_STATEMENTS, cancel);
}

/// Every statement and every loop or comprehension iteration counts once
/// (`third_party/starlark/PATCH.md`).
fn install_with(eval: &mut Evaluator<'_, '_, '_>, max_statements: u64, cancel: &CancelToken) {
    // The patched starlark checks single large strings, lists and tuples against
    // this bound before allocating them; the hook checks the heap after each statement.
    eval.heap().set_allocation_limit(Some(MAX_HEAP_BYTES));
    // Starlark 0.13 exposes the fallible statement hook through its DAP API.
    let hook: Box<dyn BeforeStmtFuncDyn<'_, '_>> = Box::new(ExecutionBudget {
        statements: 0,
        max_statements,
        started: Instant::now(),
        cancel: cancel.clone(),
    });
    eval.before_stmt_for_dap(hook.into());
}

#[cfg(test)]
mod tests {
    use super::*;
    use starlark::environment::{Globals, Module};
    use starlark::syntax::{AstModule, Dialect};

    fn run(source: &str, max_statements: u64) -> starlark::Result<()> {
        let ast = AstModule::parse("budget.star", source.to_owned(), &Dialect::Extended).unwrap();
        let module = Module::new();
        let mut eval = Evaluator::new(&module);
        install_with(&mut eval, max_statements, &CancelToken::default());
        eval.eval_module(ast, &Globals::standard()).map(|_| ())
    }

    /// Iteration without a statement in its body (comprehension clauses, a `pass`
    /// loop, a lambda's comprehension) ran past the budget: 10^10 steps took minutes.
    #[test]
    fn iteration_without_statements_counts_against_the_budget() {
        let runaways = [
            "n = len([0 for a in range(100000) for b in range(100000) if a < 0])",
            "def f():\n    for a in range(2000000000):\n        pass\nf()",
            "L = [0] * 100000\nn = len({a: 0 for a in L for b in L if b})",
            "g = lambda n: [b for b in range(n) if b < 0]\nn = [len(g(100000)) for a in range(100000)]",
        ];
        for source in runaways {
            let error = run(source, 10_000).unwrap_err();
            assert!(
                error.to_string().contains("execution budget exceeded"),
                "{source}: {error}"
            );
        }
    }

    /// Review 2026-10-11 PROG-1: one line took 2.8 GB (`"a" * 1500000000`, still
    /// "ok") or over 4 GB in 1.4 s (`list(range(300000000))`) inside the window process.
    #[test]
    fn a_program_that_builds_a_huge_value_stops_before_taking_the_memory() {
        let huge = [
            "x = \"a\" * 1500000000",
            "n = len(list(range(300000000)))",
            "n = len(tuple(range(300000000)))",
            "n = len([0] * 300000000)",
            "n = len((0,) * 300000000)",
            "s = \"a\" * 1000000\nn = len(\"\".join([s] * 2000))",
            "def f():\n    s = \"a\" * 1000000\n    for i in range(40):\n        s = s + s\n    return len(s)\nf()",
            "def f():\n    L = [0] * 1000000\n    for i in range(40):\n        L = L + L\n    return len(L)\nf()",
        ];
        for source in huge {
            let error = run(source, MAX_STATEMENTS).unwrap_err();
            assert!(
                error.to_string().contains("memory limit exceeded"),
                "{source}: {error}"
            );
        }
        run(
            "x = \"a\" * 1000000\nn = len(list(range(100000)))",
            MAX_STATEMENTS,
        )
        .unwrap();
    }

    /// Each iteration counts once, also when its body has statements.
    #[test]
    fn a_loop_costs_one_step_per_iteration_plus_its_statements() {
        let source =
            "def f():\n    t = 0\n    for a in range(1000):\n        t += a\n    return t\nf()";
        run(source, 1_010).unwrap();
        assert!(run(source, 900).is_err());
        run("n = [a for a in range(1000)]", 1_002).unwrap();
        assert!(run("n = [a for a in range(1000)]", 900).is_err());
    }
}
