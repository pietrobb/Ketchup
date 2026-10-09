//! Bounds interpreted work, including loops inside the frozen standard library.
use starlark::codemap::FileSpanRef;
use starlark::eval::{BeforeStmtFuncDyn, Evaluator};
use std::time::{Duration, Instant};

const MAX_STATEMENTS: u64 = 5_000_000;
const MAX_TIME: Duration = Duration::from_secs(20);

struct ExecutionBudget {
    statements: u64,
    max_statements: u64,
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
        if self.statements > self.max_statements
            || (self.statements.is_multiple_of(1024) && self.started.elapsed() > MAX_TIME)
        {
            return Err(starlark::Error::new_other(ExecutionLimit));
        }
        Ok(())
    }
}

pub(crate) fn install(eval: &mut Evaluator<'_, '_, '_>) {
    install_with(eval, MAX_STATEMENTS);
}

/// Every statement and every loop or comprehension iteration counts once
/// (`third_party/starlark/PATCH.md`).
fn install_with(eval: &mut Evaluator<'_, '_, '_>, max_statements: u64) {
    // Starlark 0.13 exposes the fallible statement hook through its DAP API.
    let hook: Box<dyn BeforeStmtFuncDyn<'_, '_>> = Box::new(ExecutionBudget {
        statements: 0,
        max_statements,
        started: Instant::now(),
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
        install_with(&mut eval, max_statements);
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
