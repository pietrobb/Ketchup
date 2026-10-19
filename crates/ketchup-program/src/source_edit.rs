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
    /// Bytes of the call, from its name to its closing parenthesis.
    pub bytes: Range<usize>,
    /// The first argument, when it is a string literal: the part's name.
    pub name: Option<Range<usize>>,
    /// The `color=` argument: bytes of the whole argument and of its value.
    pub color: Option<(Range<usize>, Range<usize>)>,
    /// Whole lines of the top-level statement that is only this call,
    /// `box(...)` or `x = box(...)`.
    pub statement: Option<Range<usize>>,
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
    let ast = parsed(file_name, source)?;
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
            Some((call, false)) => return Some(with_statement(&ast, source, call)),
            Some((call, true)) => {
                nested.get_or_insert(call);
            }
            None => {}
        }
    }
    nested.map(|call| with_statement(&ast, source, call))
}

fn parsed(file_name: &str, source: &str) -> Option<AstModule> {
    AstModule::parse(file_name, source.to_owned(), &crate::eval::dialect()).ok()
}

fn with_statement(ast: &AstModule, source: &str, mut call: CreatingCall) -> CreatingCall {
    call.statement = call_statement(ast, source, |_, made| span_bytes(made.span) == call.bytes);
    call
}

/// Whole lines of the top-level statement that is only a call, `f(...)` or
/// `x = f(...)`, the first that `is` picks by its statement and call.
fn call_statement(
    ast: &AstModule,
    source: &str,
    is: impl Fn(&AstStmt, &AstExpr) -> bool,
) -> Option<Range<usize>> {
    let top_level = match &ast.statement().node {
        StmtP::Statements(statements) => statements.iter().collect::<Vec<_>>(),
        _ => vec![ast.statement()],
    };
    top_level.into_iter().find_map(|statement| {
        let call = match &statement.node {
            StmtP::Expression(expression) => expression,
            StmtP::Assign(assign) => &assign.rhs,
            _ => return None,
        };
        (matches!(call.node, ExprP::Call(..)) && is(statement, call)).then(|| {
            whole_lines(
                source,
                span_bytes(statement.span).start..span_bytes(call.span).end,
            )
        })?
    })
}

fn span_bytes(span: starlark::codemap::Span) -> Range<usize> {
    span.begin().get() as usize..span.end().get() as usize
}

/// `bytes` widened to the whole lines they stand on, with the line break;
/// `None` when the lines hold anything else but a trailing comment.
fn whole_lines(source: &str, bytes: Range<usize>) -> Option<Range<usize>> {
    let start = source[..bytes.start].rfind('\n').map_or(0, |at| at + 1);
    let end = source[bytes.end..]
        .find('\n')
        .map_or(source.len(), |at| bytes.end + at + 1);
    let rest = source[bytes.end..end].trim();
    (source[start..bytes.start].trim().is_empty() && (rest.is_empty() || rest.starts_with('#')))
        .then_some(start..end)
}

/// Whole lines of the top-level statement on exactly `lines` that is a call
/// or an assignment of one, e.g. `move("a", by=(0, 0, 5))`.
#[must_use]
pub fn top_level_call_statement(
    file_name: &str,
    source: &str,
    lines: SourceLines,
) -> Option<Range<usize>> {
    let ast = parsed(file_name, source)?;
    call_statement(&ast, source, |statement, _| {
        let span = ast.codemap().resolve_span(statement.span);
        (span.begin.line + 1, span.end.line + 1) == (lines.first, lines.last)
    })
}

/// Every string literal of the source that reads `text`: its bytes, quotes
/// included, and its line.
#[must_use]
pub fn string_literals(file_name: &str, source: &str, text: &str) -> Vec<(Range<usize>, usize)> {
    let Some(ast) = parsed(file_name, source) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    walk(Visit::Stmt(ast.statement()), &mut |expression| {
        if let ExprP::Literal(AstLiteral::String(literal)) = &expression.node
            && literal.node == text
        {
            found.push((
                span_bytes(expression.span),
                ast.codemap().resolve_span(expression.span).begin.line + 1,
            ));
        }
    });
    found
}

/// The `was=` string literal of `continue_part("<part>", was=...)`.
#[must_use]
pub fn continuation_of(file_name: &str, source: &str, part: &str) -> Option<Range<usize>> {
    let ast = parsed(file_name, source)?;
    let mut found = None;
    walk(Visit::Stmt(ast.statement()), &mut |expression| {
        let ExprP::Call(callee, arguments) = &expression.node else {
            return;
        };
        if !matches!(&callee.node, ExprP::Identifier(name) if name.ident == "continue_part") {
            return;
        }
        let mut named = None;
        let mut was = None;
        for argument in &arguments.args {
            match &argument.node {
                ArgumentP::Positional(value) => {
                    if let ExprP::Literal(AstLiteral::String(literal)) = &value.node {
                        named.get_or_insert(literal.node == part);
                    }
                }
                ArgumentP::Named(name, value) if name.node == "was" => {
                    if matches!(&value.node, ExprP::Literal(AstLiteral::String(_))) {
                        was = Some(span_bytes(value.span));
                    }
                }
                _ => {}
            }
        }
        if named == Some(true) {
            found = found.take().or(was);
        }
    });
    found
}

fn walk(node: Visit<'_, AstNoPayload>, visit: &mut impl FnMut(&AstExpr)) {
    if let Visit::Expr(expression) = node {
        visit(expression);
    }
    node.visit_children(|child| walk(child, visit));
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

/// A written length or angle keeps six decimals (a nanometre, a micro-degree):
/// what the editor shows, without the noise of the matrix it came from.
#[must_use]
pub fn written(value: f64) -> f64 {
    let value = (value * 1.0e6).round() / 1.0e6;
    if value == 0.0 { 0.0 } else { value }
}

#[must_use]
pub fn written_point(point: [f64; 3]) -> [f64; 3] {
    point.map(written)
}

/// A string as a program literal, quoted and escaped.
#[must_use]
pub fn quote(text: &str) -> String {
    serde_json::to_string(text).expect("strings serialize")
}

/// Adds `lines` at the end of `source`, on a line of their own.
pub fn append_lines(source: &mut String, lines: &str) {
    if !source.is_empty() && !source.ends_with('\n') {
        source.push('\n');
    }
    source.push_str(lines);
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
                            bytes: span_bytes(expression.span),
                            name: arguments.args.first().and_then(|argument| {
                                match &argument.node {
                                    ArgumentP::Positional(value)
                                        if matches!(
                                            &value.node,
                                            ExprP::Literal(AstLiteral::String(_))
                                        ) =>
                                    {
                                        Some(span_bytes(value.span))
                                    }
                                    _ => None,
                                }
                            }),
                            color: arguments.args.iter().find_map(|argument| {
                                match &argument.node {
                                    ArgumentP::Named(name, value) if name.node == "color" => {
                                        let value = widened(self.source, span_bytes(value.span));
                                        let start = span_bytes(argument.span).start;
                                        Some((start..value.end, value))
                                    }
                                    _ => None,
                                }
                            }),
                            statement: None,
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
    Some(LiteralPoint {
        bytes: widened(source, span_bytes(expression.span)),
        value: [literal_number(x)?, literal_number(y)?, literal_number(z)?],
    })
}

/// A tuple's span leaves out its parentheses; the literal includes them.
fn widened(source: &str, bytes: Range<usize>) -> Range<usize> {
    let before = source[..bytes.start].trim_end();
    let after = source[bytes.end..].trim_start();
    if !source[bytes.start..].starts_with(['(', '['])
        && before.ends_with('(')
        && after.starts_with(')')
    {
        before.len() - 1..source.len() - after.len() + 1
    } else {
        bytes
    }
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
    fn call_names_its_name_colour_and_own_statement() {
        let source = "x = 1\ntop = box(\"a\", (10, 20, 30), color=(1, 2, 3))  # top\n\
                      pair = (box(\"b\", (1, 1, 1)), 2)\n";
        let call = call_of(source, "a").expect("found");
        assert_eq!(
            &source[call.bytes.clone()],
            "box(\"a\", (10, 20, 30), color=(1, 2, 3))"
        );
        assert_eq!(&source[call.name.expect("literal name")], "\"a\"");
        let (argument, value) = call.color.expect("colour");
        assert_eq!(&source[argument], "color=(1, 2, 3)");
        assert_eq!(&source[value], "(1, 2, 3)");
        assert_eq!(
            &source[call.statement.expect("own statement")],
            "top = box(\"a\", (10, 20, 30), color=(1, 2, 3))  # top\n"
        );
        assert_eq!(call_of(source, "b").expect("found").statement, None);
        let lines = SourceLines { first: 2, last: 2 };
        assert_eq!(
            top_level_call_statement("p.star", source, lines).map(|bytes| &source[bytes]),
            Some("top = box(\"a\", (10, 20, 30), color=(1, 2, 3))  # top\n")
        );
        let literals = string_literals("p.star", source, "b");
        assert_eq!(literals.len(), 1);
        assert_eq!(
            (&source[literals[0].0.clone()], literals[0].1),
            ("\"b\"", 3)
        );
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
