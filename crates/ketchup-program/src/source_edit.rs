//! Finds the call in a program's source that creates a part, so an edit made
//! in the editor can change that call in place instead of adding lines that
//! only undo what an earlier line did.
use crate::SourceLines;
use starlark::codemap::CodeMap;
use starlark::syntax::AstModule;
use starlark_syntax::lexer::TokenInt;
use starlark_syntax::syntax::ast::{
    ArgumentP, AstExpr, AstLiteral, AstNoPayload, AstStmt, ExprP, StmtP,
};
use starlark_syntax::syntax::module::AstModuleFields;
use starlark_syntax::syntax::uniplate::Visit;
use std::collections::BTreeSet;
use std::ops::Range;

/// The call of the user's program a part's evaluation started from.
#[derive(Clone, Debug, PartialEq)]
pub struct CreatingCall {
    pub lines: SourceLines,
    /// First line of the loop the call runs in once per pass.
    pub loop_line: Option<usize>,
    /// The program's own function the call is made in or calls; it may
    /// create other parts too.
    pub function: Option<String>,
    /// `at=` of the call, when it is three number literals.
    pub at: Option<LiteralPoint>,
}

/// Three number literals in the source, e.g. `(0, 18, -5.5)`.
#[derive(Clone, Debug, PartialEq)]
pub struct LiteralPoint {
    /// Bytes of the literal in the source.
    pub bytes: Range<usize>,
    pub value: [f64; 3],
}

/// The outermost call of the program that created a part, from the part's
/// recorded program lines in order (every call on the way to each builtin
/// that made or changed it): the first call made outside the program's own
/// functions. `None` when the source does not parse or no call spans them.
#[must_use]
pub fn creating_call(
    file_name: &str,
    source: &str,
    recorded: &[SourceLines],
) -> Option<CreatingCall> {
    let ast = AstModule::parse(file_name, source.to_owned(), &crate::eval::dialect()).ok()?;
    let mut functions = BTreeSet::new();
    collect_functions(ast.statement(), &mut functions);
    let mut nested = None;
    for &lines in recorded {
        let mut search = Search {
            codemap: ast.codemap(),
            source,
            functions: &functions,
            lines,
            found: None,
        };
        search.visit(Visit::Stmt(ast.statement()), &Context::default());
        match search.found {
            Some((call, false)) => return Some(call),
            Some((call, true)) => {
                nested.get_or_insert(call);
            }
            None => {}
        }
    }
    nested
}

/// A number as a program literal: shortest exact form, no `-0`.
#[must_use]
pub fn number_literal(value: f64) -> String {
    if value == 0.0 {
        "0".to_owned()
    } else {
        value.to_string()
    }
}

/// `(x, y, z)` as a program literal.
#[must_use]
pub fn point_literal(point: [f64; 3]) -> String {
    format!(
        "({}, {}, {})",
        number_literal(point[0]),
        number_literal(point[1]),
        number_literal(point[2])
    )
}

#[derive(Clone, Default)]
struct Context {
    loop_line: Option<usize>,
    function: Option<String>,
}

struct Search<'a> {
    codemap: &'a CodeMap,
    source: &'a str,
    functions: &'a BTreeSet<String>,
    lines: SourceLines,
    /// The call, and whether it is made inside a function of the program.
    found: Option<(CreatingCall, bool)>,
}

impl Search<'_> {
    fn line(&self, span: starlark::codemap::Span) -> usize {
        self.codemap.resolve_span(span).begin.line + 1
    }

    fn visit(&mut self, node: Visit<'_, AstNoPayload>, context: &Context) {
        if self.found.is_some() {
            return;
        }
        let mut inner = context.clone();
        match node {
            Visit::Stmt(statement) => match &statement.node {
                StmtP::For(_) => inner.loop_line = Some(self.line(statement.span)),
                StmtP::Def(def) => inner.function = Some(def.name.ident.clone()),
                _ => {}
            },
            Visit::Expr(expression) => match &expression.node {
                ExprP::ListComprehension(..) | ExprP::DictComprehension(..) => {
                    inner.loop_line = Some(self.line(expression.span));
                }
                ExprP::Lambda(_) => inner.function = Some("lambda".to_owned()),
                ExprP::Call(callee, arguments) => {
                    let span = self.codemap.resolve_span(expression.span);
                    if (span.begin.line + 1, span.end.line + 1)
                        == (self.lines.first, self.lines.last)
                    {
                        let called = match &callee.node {
                            ExprP::Identifier(name) if self.functions.contains(&name.ident) => {
                                Some(name.ident.clone())
                            }
                            _ => None,
                        };
                        let call = CreatingCall {
                            lines: self.lines,
                            loop_line: context.loop_line,
                            function: context.function.clone().or(called),
                            at: arguments
                                .args
                                .iter()
                                .find_map(|argument| match &argument.node {
                                    ArgumentP::Named(name, value) if name.node == "at" => {
                                        literal_point(self.source, value)
                                    }
                                    _ => None,
                                }),
                        };
                        self.found = Some((call, context.function.is_some()));
                        return;
                    }
                }
                _ => {}
            },
        }
        node.visit_children(|child| self.visit(child, &inner));
    }
}

fn collect_functions(statement: &AstStmt, functions: &mut BTreeSet<String>) {
    if let StmtP::Def(def) = &statement.node {
        functions.insert(def.name.ident.clone());
    }
    statement.visit_stmt(|child| collect_functions(child, functions));
}

fn literal_point(source: &str, expression: &AstExpr) -> Option<LiteralPoint> {
    let (ExprP::Tuple(items) | ExprP::List(items)) = &expression.node else {
        return None;
    };
    let [x, y, z] = items.as_slice() else {
        return None;
    };
    let (mut start, mut end) = (
        expression.span.begin().get() as usize,
        expression.span.end().get() as usize,
    );
    // A tuple's span leaves out its parentheses; the literal includes them.
    let before = source[..start].trim_end();
    let after = source[end..].trim_start();
    if !source[start..].starts_with(['(', '[']) && before.ends_with('(') && after.starts_with(')') {
        start = before.len() - 1;
        end = source.len() - after.len() + 1;
    }
    Some(LiteralPoint {
        bytes: start..end,
        value: [literal_number(x)?, literal_number(y)?, literal_number(z)?],
    })
}

fn literal_number(expression: &AstExpr) -> Option<f64> {
    match &expression.node {
        ExprP::Literal(AstLiteral::Int(value)) => match &value.node {
            TokenInt::I32(value) => Some(f64::from(*value)),
            TokenInt::BigInt(_) => None,
        },
        ExprP::Literal(AstLiteral::Float(value)) => Some(value.node),
        ExprP::Minus(inner) => literal_number(inner).map(|value| -value),
        ExprP::Plus(inner) => literal_number(inner),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn call_of(source: &str, part: &str) -> Option<CreatingCall> {
        let evaluated = crate::evaluate("p.star", source, &BTreeMap::new()).expect("evaluates");
        creating_call("p.star", source, &evaluated.part_sources[part])
    }

    #[test]
    fn direct_call_names_its_literal_position() {
        let source = "x = 10\nbox(\"a\", (100, 18, 400),\n    at=(0, -5.5, 2))\n";
        let call = call_of(source, "a").expect("found");
        assert_eq!(call.lines, SourceLines { first: 2, last: 3 });
        assert_eq!(call.loop_line, None);
        assert_eq!(call.function, None);
        let at = call.at.expect("literal at");
        assert_eq!(at.value, [0.0, -5.5, 2.0]);
        assert_eq!(&source[at.bytes], "(0, -5.5, 2)");
    }

    #[test]
    fn loop_and_own_function_are_named() {
        let source = "def shelf(name, z):\n    box(name, (100, 18, 400), at=(0, 0, z))\n\
                      for i in range(2):\n    shelf(\"s%d\" % i, 100 * i)\n\
                      shelf(\"top\", 500)\n";
        let looped = call_of(source, "s1").expect("found");
        assert_eq!(looped.loop_line, Some(3));
        assert_eq!(looped.function.as_deref(), Some("shelf"));
        let top = call_of(source, "top").expect("found");
        assert_eq!(top.loop_line, None);
        assert_eq!(top.function.as_deref(), Some("shelf"));
        assert_eq!(top.at, None);
    }

    #[test]
    fn computed_position_is_not_a_literal() {
        let source = "x = 10\nbox(\"a\", (10, 20, 30), at=(x, 0, 0))\n";
        let call = call_of(source, "a").expect("found");
        assert_eq!(call.at, None);
        let listed = "box(\"a\", (10, 20, 30), at=[1, 2, 3])\nmove(\"a\", by=(0, 0, 9))\n";
        let at = call_of(listed, "a").expect("found").at.expect("literal at");
        assert_eq!(&listed[at.bytes], "[1, 2, 3]");
    }
}
