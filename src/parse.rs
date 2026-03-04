//! Recursive-descent parser.
//!
//! Precedence, tightest to loosest:
//! ```text
//!     postfix   . [ ] ? call
//!     unary     - not
//!     mul       * / %
//!     add       + -
//!     cmp       == != < <= > >=
//!     and
//!     or
//!     assign    = |=
//!     alt       //
//!     comma     ,
//!     pipe      |
//! ```
//!
//! Selector-shortcut pseudos (`:first`, `:nth(k)`, `:lang(x)`) are
//! handled in `parse_postfix` and desugar to plain jq inline. No
//! separate desugar pass.

use std::sync::Arc;

use crate::error::CompileError;
use crate::expr::{AssignOp, BinOp, CmpOp, Expr, Literal, ObjKey};
use crate::lex::{Spanned, Tok};

/// Parse a token slice into a top-level [`Expr`].
pub fn parse(tokens: &[Spanned<'_>]) -> Result<Expr, CompileError> {
    let mut p = Parser::new(tokens);
    let expr = p.parse_top()?;
    if !matches!(p.peek(), Tok::Eof) {
        return p.err("end of input");
    }
    // If the query used `>` combinators, the desugaring references
    // `$__root` so `section(...)` can run against the original root
    // instead of whichever node the chain flowed into. Bind it here.
    if p.chain_var > 0 {
        Ok(Expr::As {
            bind: Box::new(Expr::Identity),
            name: Arc::from("__root"),
            body: Box::new(expr),
        })
    } else {
        Ok(expr)
    }
}

struct Parser<'t, 's> {
    toks: &'t [Spanned<'s>],
    pos: usize,
    /// Counter for fresh `$__tN` binding names used by the `>`
    /// selector combinator desugaring.
    chain_var: usize,
}

impl<'t, 's> Parser<'t, 's> {
    fn new(toks: &'t [Spanned<'s>]) -> Self {
        Self { toks, pos: 0, chain_var: 0 }
    }

    fn peek(&self) -> &Tok<'s> {
        &self.toks[self.pos].tok
    }

    /// Look `n` tokens ahead without consuming. Used by the selector
    /// chain parser to decide whether a `>` is a combinator or the
    /// greater-than operator.
    fn peek_n(&self, n: usize) -> &Tok<'s> {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)].tok
    }

    fn peek_offset(&self) -> usize {
        self.toks[self.pos].offset
    }

    fn advance(&mut self) -> &Spanned<'s> {
        let t = &self.toks[self.pos];
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn err<T>(&self, expected: &str) -> Result<T, CompileError> {
        Err(CompileError::Parse {
            offset: self.peek_offset(),
            expected: expected.into(),
            found: describe(self.peek()),
        })
    }

    /// Peel off leading `def ...; def ...;` forms, then parse the
    /// main pipeline against the resulting scope.
    fn parse_top(&mut self) -> Result<Expr, CompileError> {
        if !matches!(self.peek(), Tok::KwDef) {
            return self.parse_pipeline();
        }
        self.advance();
        let Tok::Ident(name) = self.peek().clone() else { return self.err("name after `def`"); };
        self.advance();
        let params = self.parse_def_params()?;
        self.expect(Tok::Colon, "`:`")?;
        // Body uses parse_top so a nested `def` parses; falling through
        // to parse_pipeline would reject KwDef as an unknown token.
        let body = self.parse_top()?;
        self.expect(Tok::Semicolon, "`;`")?;
        let rest = self.parse_top()?;
        Ok(Expr::Def { name: Arc::from(name), params, body: Box::new(body), rest: Box::new(rest) })
    }

    /// Parse the optional `(p1; p2; ...)` after a def name.
    fn parse_def_params(&mut self) -> Result<Vec<Arc<str>>, CompileError> {
        if !matches!(self.peek(), Tok::LParen) {
            return Ok(Vec::new());
        }
        self.advance();
        let mut params = Vec::new();
        while !matches!(self.peek(), Tok::RParen) {
            let Tok::Ident(n) = self.peek().clone() else { return self.err("parameter name"); };
            self.advance();
            params.push(Arc::<str>::from(n));
            if !matches!(self.peek(), Tok::Semicolon) { break; }
            self.advance();
        }
        self.expect(Tok::RParen, "`)`")?;
        Ok(params)
    }

    // ---- precedence layers -------------------------------------------------

    fn parse_pipeline(&mut self) -> Result<Expr, CompileError> {
        self.parse_pipeline_with(Self::parse_comma, true)
    }

    /// Pipeline without comma-union. Used inside `{...}` entries and
    /// `fn(a; b)` args, where `,` is a separator instead of an operator.
    fn parse_pipeline_no_comma(&mut self) -> Result<Expr, CompileError> {
        self.parse_pipeline_with(Self::parse_alt, false)
    }

    fn parse_pipeline_with(&mut self, next: fn(&mut Self) -> Result<Expr, CompileError>, allow_comma: bool) -> Result<Expr, CompileError> {
        let mut lhs = next(self)?;
        loop {
            match self.peek() {
                Tok::Pipe => {
                    self.advance();
                    lhs = Expr::Pipe(Box::new(lhs), Box::new(next(self)?));
                }
                Tok::KwAs => lhs = self.parse_as_tail(lhs, allow_comma)?,
                _ => break,
            }
        }
        Ok(lhs)
    }

    /// Parse `bind as $name | body`. `bind` already consumed.
    fn parse_as_tail(&mut self, bind: Expr, allow_comma: bool) -> Result<Expr, CompileError> {
        self.advance();
        let Tok::DollarIdent(name) = self.peek().clone() else { return self.err("`$name` after `as`"); };
        self.advance();
        self.expect(Tok::Pipe, "`|` after `as $name`")?;
        let body = if allow_comma { self.parse_comma()? } else { self.parse_alt()? };
        Ok(Expr::As { bind: Box::new(bind), name: Arc::from(name), body: Box::new(body) })
    }

    fn parse_comma(&mut self) -> Result<Expr, CompileError> {
        let mut lhs = self.parse_alt()?;
        while matches!(self.peek(), Tok::Comma) {
            self.advance();
            let rhs = self.parse_alt()?;
            lhs = Expr::Comma(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_alt(&mut self) -> Result<Expr, CompileError> {
        let mut lhs = self.parse_assign()?;
        while matches!(self.peek(), Tok::SlashSlash) {
            self.advance();
            let rhs = self.parse_assign()?;
            lhs = Expr::Bin(Box::new(lhs), BinOp::Alt, Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_assign(&mut self) -> Result<Expr, CompileError> {
        let lhs = self.parse_or()?;
        let op = match self.peek() {
            Tok::Eq => Some(AssignOp::Set),
            Tok::PipeEq => Some(AssignOp::Update),
            _ => None,
        };
        if let Some(op) = op {
            self.advance();
            let rhs = self.parse_or()?;
            return Ok(Expr::Assign(Box::new(lhs), op, Box::new(rhs)));
        }
        Ok(lhs)
    }

    fn parse_or(&mut self) -> Result<Expr, CompileError> {
        self.left_assoc_bin(Self::parse_and, |t| matches!(t, Tok::KwOr).then_some(BinOp::Or))
    }

    fn parse_and(&mut self) -> Result<Expr, CompileError> {
        self.left_assoc_bin(Self::parse_cmp, |t| matches!(t, Tok::KwAnd).then_some(BinOp::And))
    }

    fn parse_cmp(&mut self) -> Result<Expr, CompileError> {
        let lhs = self.parse_add()?;
        let Some(op) = cmp_op(self.peek()) else {
            return Ok(lhs);
        };
        self.advance();
        let rhs = self.parse_add()?;
        Ok(Expr::Cmp(Box::new(lhs), op, Box::new(rhs)))
    }

    fn parse_add(&mut self) -> Result<Expr, CompileError> {
        self.left_assoc_bin(Self::parse_mul, add_op)
    }

    fn parse_mul(&mut self) -> Result<Expr, CompileError> {
        self.left_assoc_bin(Self::parse_unary, mul_op)
    }

    /// Left-associative binop chain: `next (op next)*`. Shared by
    /// `or`, `and`, `add`, `mul`.
    fn left_assoc_bin(
        &mut self,
        next: fn(&mut Self) -> Result<Expr, CompileError>,
        op_for: impl Fn(&Tok<'s>) -> Option<BinOp>,
    ) -> Result<Expr, CompileError> {
        let mut lhs = next(self)?;
        while let Some(op) = op_for(self.peek()) {
            self.advance();
            let rhs = next(self)?;
            lhs = Expr::Bin(Box::new(lhs), op, Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, CompileError> {
        let wrap: fn(Box<Expr>) -> Expr = match self.peek() {
            Tok::Minus => Expr::Neg,
            Tok::KwNot => Expr::Not,
            _ => return self.parse_postfix(),
        };
        self.advance();
        Ok(wrap(Box::new(self.parse_unary()?)))
    }

    fn parse_postfix(&mut self) -> Result<Expr, CompileError> {
        let mut lhs = self.parse_primary()?;
        loop {
            match self.peek() {
                Tok::Dot => {
                    let dot_off = self.peek_offset();
                    self.advance();
                    match self.peek().clone() {
                        Tok::Ident(name) if self.peek_offset() == dot_off + 1 => {
                            self.advance();
                            lhs = Expr::Pipe(
                                Box::new(lhs),
                                Box::new(Expr::Field(Arc::from(name))),
                            );
                        }
                        Tok::LBracket if self.peek_offset() == dot_off + 1 => {
                            lhs = self.parse_bracket_access(lhs)?;
                        }
                        _ => return self.err("identifier or `[` after `.`"),
                    }
                }
                Tok::LBracket => {
                    lhs = self.parse_bracket_access(lhs)?;
                }
                Tok::Question => {
                    self.advance();
                    lhs = Expr::Try(Box::new(lhs));
                }
                Tok::ColonIdent(pseudo) => {
                    let pseudo = *pseudo;
                    self.advance();
                    lhs = self.apply_pseudo(lhs, pseudo)?;
                }
                // `>` as a selector combinator. Only when followed by
                // another selector origin; otherwise it stays for
                // parse_cmp to read as `Gt`.
                Tok::Gt if looks_like_selector_start(self.peek_n(1)) => {
                    self.advance();
                    let rhs = self.parse_postfix()?;
                    lhs = self.combine_sections(lhs, rhs);
                }
                _ => break,
            }
        }
        Ok(lhs)
    }

    /// Desugar `lhs > rhs` into
    /// `lhs | [headings] | .[0] | .text as $__tN | $__root | section($__tN) | rhs`.
    ///
    /// `[headings] | .[0]` normalises whatever `lhs` produced (heading
    /// directly, or section containing one) to the first heading, so
    /// `.text` always reads the title. `$__root` is bound at parse-top
    /// when any combinator appears.
    fn combine_sections(&mut self, lhs: Expr, rhs: Expr) -> Expr {
        let var = Arc::<str>::from(format!("__t{}", self.chain_var));
        self.chain_var += 1;

        let first_heading = Expr::Pipe(
            Box::new(Expr::ArrayCtor(Box::new(Expr::Call {
                name: Arc::from("headings"),
                args: Vec::new(),
            }))),
            Box::new(Expr::Index(Box::new(Expr::Lit(Literal::Number(0.0))))),
        );

        let section_call = Expr::Call {
            name: Arc::from("section"),
            args: vec![Expr::Var(var.clone())],
        };
        let body_tail = Expr::Pipe(
            Box::new(Expr::Var(Arc::from("__root"))),
            Box::new(Expr::Pipe(Box::new(section_call), Box::new(rhs))),
        );
        let binding = Expr::As {
            bind: Box::new(Expr::Field(Arc::from("text"))),
            name: var,
            body: Box::new(body_tail),
        };

        Expr::Pipe(
            Box::new(lhs),
            Box::new(Expr::Pipe(Box::new(first_heading), Box::new(binding))),
        )
    }

    /// Selector pseudos desugar inline:
    /// ```text
    ///   E:first    ->  [E] | .[0]
    ///   E:last     ->  [E] | .[-1]
    ///   E:nth(k)   ->  [E] | .[k]
    ///   E:lang(x)  ->  E  | select(.lang == x)
    ///   E:text(x)  ->  E  | select(.text == x)
    /// ```
    fn apply_pseudo(&mut self, lhs: Expr, pseudo: &'s str) -> Result<Expr, CompileError> {
        match pseudo {
            "first" => Ok(wrap_index(lhs, Expr::Lit(Literal::Number(0.0)))),
            "last" => Ok(wrap_index(lhs, Expr::Lit(Literal::Number(-1.0)))),
            "nth" => {
                let k = self.parse_paren_arg("`:nth`")?;
                Ok(wrap_index(lhs, k))
            }
            "lang" => {
                let rhs = self.parse_ident_or_string_arg("`:lang`")?;
                Ok(select_on_field_eq(lhs, "lang", rhs))
            }
            "text" => {
                let rhs = self.parse_ident_or_string_arg("`:text`")?;
                Ok(select_on_field_eq(lhs, "text", rhs))
            }
            other => Err(CompileError::Selector {
                offset: self.peek_offset(),
                message: format!("unknown pseudo `:{other}`"),
            }),
        }
    }

    fn parse_bracket_access(&mut self, lhs: Expr) -> Result<Expr, CompileError> {
        // Peek confirmed `[`. Consume it now.
        debug_assert!(matches!(self.peek(), Tok::LBracket));
        self.advance();

        // `[]` -> iterate
        if matches!(self.peek(), Tok::RBracket) {
            self.advance();
            return Ok(Expr::Pipe(Box::new(lhs), Box::new(Expr::Iterate)));
        }

        // `[:b]`: slice with no start.
        if matches!(self.peek(), Tok::Colon) {
            self.advance();
            let end = if matches!(self.peek(), Tok::RBracket) {
                None
            } else {
                Some(Box::new(self.parse_pipeline()?))
            };
            self.expect_rbracket()?;
            return Ok(Expr::Pipe(
                Box::new(lhs),
                Box::new(Expr::Slice(None, end)),
            ));
        }

        let first = self.parse_pipeline()?;
        if matches!(self.peek(), Tok::Colon) {
            self.advance();
            let end = if matches!(self.peek(), Tok::RBracket) {
                None
            } else {
                Some(Box::new(self.parse_pipeline()?))
            };
            self.expect_rbracket()?;
            return Ok(Expr::Pipe(
                Box::new(lhs),
                Box::new(Expr::Slice(Some(Box::new(first)), end)),
            ));
        }
        self.expect_rbracket()?;
        Ok(Expr::Pipe(Box::new(lhs), Box::new(Expr::Index(Box::new(first)))))
    }

    /// Consume `( expr )`. `label` shows up in the `expected (` error.
    fn parse_paren_arg(&mut self, label: &str) -> Result<Expr, CompileError> {
        self.expect(Tok::LParen, &format!("`(` after {label}"))?;
        let arg = self.parse_pipeline_no_comma()?;
        self.expect(Tok::RParen, "`)`")?;
        Ok(arg)
    }

    /// Like `parse_paren_arg`, but a bare ident in the arg slot is
    /// read as its string literal. Used by `:lang(rust)` / `:text(foo)`,
    /// where the ident would otherwise dispatch as a builtin.
    fn parse_ident_or_string_arg(&mut self, label: &str) -> Result<Expr, CompileError> {
        self.expect(Tok::LParen, &format!("`(` after {label}"))?;
        let arg = match self.peek().clone() {
            Tok::Ident(name) if matches!(self.peek_n(1), Tok::RParen) => {
                self.advance();
                Expr::Lit(Literal::String(Arc::from(name)))
            }
            _ => self.parse_pipeline_no_comma()?,
        };
        self.expect(Tok::RParen, "`)`")?;
        Ok(arg)
    }

    fn expect_rbracket(&mut self) -> Result<(), CompileError> {
        if matches!(self.peek(), Tok::RBracket) {
            self.advance();
            Ok(())
        } else {
            self.err("`]`")
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, CompileError> {
        match self.peek().clone() {
            Tok::Dot => {
                let dot_off = self.peek_offset();
                self.advance();
                if let Tok::Ident(name) = self.peek().clone() {
                    if self.peek_offset() == dot_off + 1 {
                        self.advance();
                        return Ok(Expr::Field(Arc::from(name)));
                    }
                }
                Ok(Expr::Identity)
            }
            Tok::DotDot => {
                self.advance();
                Ok(Expr::RecurseAll)
            }
            Tok::LParen => {
                self.advance();
                let inner = self.parse_pipeline()?;
                self.expect(Tok::RParen, "`)`")?;
                Ok(inner)
            }
            Tok::LBracket => {
                self.advance();
                if matches!(self.peek(), Tok::RBracket) {
                    self.advance();
                    // `[]` is the empty array, not `[.]`. Wrap the
                    // `empty` builtin so the array ctor sees a stream
                    // with no items.
                    return Ok(Expr::ArrayCtor(Box::new(Expr::Call {
                        name: Arc::from("empty"),
                        args: Vec::new(),
                    })));
                }
                let inner = self.parse_pipeline()?;
                self.expect(Tok::RBracket, "`]`")?;
                Ok(Expr::ArrayCtor(Box::new(inner)))
            }
            Tok::LBrace => self.parse_object_ctor(),
            Tok::Str(value) => {
                self.advance();
                build_string_literal(value.as_ref())
            }
            Tok::Num(n) => {
                self.advance();
                Ok(Expr::Lit(Literal::Number(n)))
            }
            Tok::KwTrue => self.advance_with(Expr::Lit(Literal::Bool(true))),
            Tok::KwFalse => self.advance_with(Expr::Lit(Literal::Bool(false))),
            Tok::KwNull => self.advance_with(Expr::Lit(Literal::Null)),
            Tok::DollarIdent(name) => self.advance_with(Expr::Var(Arc::from(name))),
            Tok::Ident("reduce") => {
                self.advance();
                self.parse_reduce()
            }
            Tok::Ident("foreach") => {
                self.advance();
                self.parse_foreach()
            }
            Tok::Ident(name) => {
                self.advance();
                let args = if matches!(self.peek(), Tok::LParen) {
                    self.advance();
                    let a = self.parse_args()?;
                    self.expect(Tok::RParen, "`)`")?;
                    a
                } else {
                    Vec::new()
                };
                Ok(Expr::Call { name: Arc::from(name), args })
            }
            Tok::KwIf => self.parse_if(),
            Tok::Hash(level) => {
                self.advance();
                self.parse_hash_selector(level)
            }
            _ => self.err("expression"),
        }
    }

    #[allow(clippy::unnecessary_wraps)]
    fn advance_with(&mut self, expr: Expr) -> Result<Expr, CompileError> {
        self.advance();
        Ok(expr)
    }

    fn parse_args(&mut self) -> Result<Vec<Expr>, CompileError> {
        if matches!(self.peek(), Tok::RParen) {
            return Ok(Vec::new());
        }
        let mut args = vec![self.parse_pipeline()?];
        while matches!(self.peek(), Tok::Semicolon) {
            self.advance();
            args.push(self.parse_pipeline()?);
        }
        Ok(args)
    }

    fn parse_object_ctor(&mut self) -> Result<Expr, CompileError> {
        debug_assert!(matches!(self.peek(), Tok::LBrace));
        self.advance();

        let mut entries = Vec::new();
        if !matches!(self.peek(), Tok::RBrace) {
            loop {
                let (key, shorthand) = self.parse_obj_key()?;
                let value = if matches!(self.peek(), Tok::Colon) {
                    self.advance();
                    self.parse_pipeline_no_comma()?
                } else if shorthand {
                    // `{foo}` means `{foo: .foo}`.
                    if let ObjKey::Ident(name) = &key {
                        Expr::Field(name.clone())
                    } else {
                        return self.err("`:` after non-identifier key");
                    }
                } else {
                    return self.err("`:` after key");
                };
                entries.push((key, value));
                match self.peek() {
                    Tok::Comma => {
                        self.advance();
                    }
                    Tok::RBrace => break,
                    _ => return self.err("`,` or `}`"),
                }
            }
        }
        self.expect(Tok::RBrace, "`}`")?;
        Ok(Expr::ObjectCtor(entries))
    }

    fn parse_obj_key(&mut self) -> Result<(ObjKey, bool), CompileError> {
        match self.peek().clone() {
            Tok::Ident(name) => {
                self.advance();
                Ok((ObjKey::Ident(Arc::from(name)), true))
            }
            Tok::Str(value) => {
                self.advance();
                Ok((ObjKey::Str(Arc::from(value.as_ref())), false))
            }
            Tok::LParen => {
                self.advance();
                let inner = self.parse_pipeline()?;
                self.expect(Tok::RParen, "`)`")?;
                Ok((ObjKey::Expr(inner), false))
            }
            _ => self.err("object key"),
        }
    }

    fn parse_if(&mut self) -> Result<Expr, CompileError> {
        debug_assert!(matches!(self.peek(), Tok::KwIf));
        self.advance();
        let mut branches = vec![self.parse_then_clause()?];
        while matches!(self.peek(), Tok::KwElif) {
            self.advance();
            branches.push(self.parse_then_clause()?);
        }
        let else_branch = matches!(self.peek(), Tok::KwElse)
            .then(|| {
                self.advance();
                self.parse_pipeline().map(Box::new)
            })
            .transpose()?;
        self.expect(Tok::KwEnd, "`end`")?;
        Ok(Expr::If { branches, else_branch })
    }

    /// Parse `cond then branch`. Used once for `if` and N times for `elif`.
    fn parse_then_clause(&mut self) -> Result<(Expr, Expr), CompileError> {
        let cond = self.parse_pipeline()?;
        self.expect(Tok::KwThen, "`then`")?;
        let then_branch = self.parse_pipeline()?;
        Ok((cond, then_branch))
    }

    /// Parse the shorthand selector that starts with `#..######`.
    /// The `Hash(level)` token has already been consumed.
    ///
    /// * `# Title` and `# "Multi word"` desugar to `section("...")`.
    /// * `#..######` with nothing following maps to the matching
    ///   `hN` kind filter.
    #[allow(clippy::unnecessary_wraps)] // shares the `Result` return of other parse_* fns.
    fn parse_hash_selector(&mut self, level: u8) -> Result<Expr, CompileError> {
        match self.peek().clone() {
            Tok::Ident(title) => {
                self.advance();
                Ok(section_call(title))
            }
            Tok::Str(title) => {
                self.advance();
                Ok(section_call(title.as_ref()))
            }
            _ => Ok(Expr::Call {
                name: Arc::from(format!("h{level}").as_str()),
                args: Vec::new(),
            }),
        }
    }

    /// Parse `reduce SRC as $x (INIT; UPDATE)`. The `reduce` ident is
    /// already consumed.
    fn parse_reduce(&mut self) -> Result<Expr, CompileError> {
        let (source, var, mut parts) = self.parse_fold_head(2)?;
        Ok(Expr::Reduce {
            source: Box::new(source),
            var,
            update: Box::new(parts.remove(1)),
            init: Box::new(parts.remove(0)),
        })
    }

    /// Parse `foreach SRC as $x (INIT; UPDATE; EXTRACT)`.
    fn parse_foreach(&mut self) -> Result<Expr, CompileError> {
        let (source, var, mut parts) = self.parse_fold_head(3)?;
        Ok(Expr::Foreach {
            source: Box::new(source),
            var,
            extract: Box::new(parts.remove(2)),
            update: Box::new(parts.remove(1)),
            init: Box::new(parts.remove(0)),
        })
    }

    /// Shared head: `SRC as $x ( P1; P2; ... )`. Returns the source,
    /// variable name, and `n_parts` parenthesised sub-expressions.
    fn parse_fold_head(&mut self, n_parts: usize) -> Result<(Expr, Arc<str>, Vec<Expr>), CompileError> {
        let source = self.parse_alt()?;
        let var = self.expect_as_var()?;
        self.expect(Tok::LParen, "`(`")?;
        let mut parts = Vec::with_capacity(n_parts);
        for i in 0..n_parts {
            if i > 0 {
                self.expect(Tok::Semicolon, "`;`")?;
            }
            parts.push(self.parse_pipeline_no_comma()?);
        }
        self.expect(Tok::RParen, "`)`")?;
        Ok((source, var, parts))
    }

    /// Expect `as $name` and return the name.
    fn expect_as_var(&mut self) -> Result<Arc<str>, CompileError> {
        self.expect(Tok::KwAs, "`as`")?;
        let Tok::DollarIdent(n) = self.peek().clone() else { return self.err("`$name`"); };
        self.advance();
        Ok(Arc::from(n))
    }

    /// Assert the current token matches `expected`. Consumes on match.
    fn expect(&mut self, expected: Tok<'_>, label: &str) -> Result<(), CompileError> {
        if std::mem::discriminant(self.peek()) == std::mem::discriminant(&expected) {
            self.advance();
            Ok(())
        } else {
            self.err(label)
        }
    }
}

fn describe(tok: &Tok<'_>) -> String {
    format!("{tok:?}")
}

/// Recognise tokens that open a selector segment. Used by the `>`
/// combinator to distinguish `heading > codeblocks` from `x > 5`.
fn looks_like_selector_start(tok: &Tok<'_>) -> bool {
    matches!(
        tok,
        Tok::Hash(_)
            | Tok::Ident(
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                | "headings" | "paragraphs" | "codeblocks" | "code"
                | "links" | "images" | "items" | "lists" | "tables"
                | "blockquotes" | "footnotes"
            )
    )
}

/// `section("title")` as an Expr. Used by the `#`-selector shorthand.
fn section_call(title: &str) -> Expr {
    Expr::Call {
        name: Arc::from("section"),
        args: vec![Expr::Lit(Literal::String(Arc::from(title)))],
    }
}

/// Build `[E] | .[index]`. Backs `:first`, `:last`, and `:nth(k)`.
fn wrap_index(expr: Expr, index: Expr) -> Expr {
    Expr::Pipe(
        Box::new(Expr::ArrayCtor(Box::new(expr))),
        Box::new(Expr::Index(Box::new(index))),
    )
}

/// `E | select(.FIELD == RHS)`.
fn select_on_field_eq(lhs: Expr, field: &str, rhs: Expr) -> Expr {
    let cmp = Expr::Cmp(
        Box::new(Expr::Field(Arc::from(field))),
        CmpOp::Eq,
        Box::new(rhs),
    );
    Expr::Pipe(
        Box::new(lhs),
        Box::new(Expr::Call {
            name: Arc::from("select"),
            args: vec![cmp],
        }),
    )
}

/// Turn a string-literal body into an Expr. If it contains `\(expr)`
/// markers, build `"..." + tostring(expr) + "..."` chains. Each
/// interpolated expression is re-lexed + re-parsed from the body.
fn build_string_literal(raw: &str) -> Result<Expr, CompileError> {
    if !raw.contains("\\(") {
        return Ok(Expr::Lit(Literal::String(Arc::from(raw))));
    }
    let mut parts: Vec<Expr> = Vec::new();
    let bytes = raw.as_bytes();
    let mut literal_start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && bytes.get(i + 1) == Some(&b'(') {
            if i > literal_start {
                parts.push(lit_str(&raw[literal_start..i]));
            }
            let expr_start = i + 2;
            let close = find_matching_paren(bytes, expr_start).ok_or_else(|| {
                CompileError::Lex {
                    offset: 0,
                    message: "unterminated `\\(` in string literal".into(),
                }
            })?;
            let inner = &raw[expr_start..close];
            let toks = crate::lex::tokenize(inner)?;
            let inner_expr = parse(&toks)?;
            parts.push(Expr::Pipe(
                Box::new(inner_expr),
                Box::new(Expr::Call {
                    name: Arc::from("tostring"),
                    args: Vec::new(),
                }),
            ));
            i = close + 1;
            literal_start = i;
        } else {
            i += 1;
        }
    }
    if literal_start < raw.len() {
        parts.push(lit_str(&raw[literal_start..]));
    }
    if parts.is_empty() {
        return Ok(lit_str(""));
    }
    let mut iter = parts.into_iter();
    let mut result = iter.next().unwrap();
    for part in iter {
        result = Expr::Bin(Box::new(result), BinOp::Add, Box::new(part));
    }
    // If the first part was an interpolation, the expression evaluates
    // to the tostring() output directly. Prepend "" so the type anchors
    // to String (matters when tostring returns a non-string-compatible
    // value through some mis-wiring; cheap insurance).
    if !matches!(result, Expr::Lit(Literal::String(_))) {
        result = Expr::Bin(Box::new(lit_str("")), BinOp::Add, Box::new(result));
    }
    Ok(result)
}

fn lit_str(s: &str) -> Expr {
    Expr::Lit(Literal::String(Arc::from(s)))
}

/// Walk forward from `start` and return the index of the `)` that
/// closes the `\(` call. Respects nested parens and string literals.
fn find_matching_paren(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut in_str = false;
    let mut j = start;
    while j < bytes.len() {
        let c = bytes[j];
        if in_str {
            if c == b'\\' && j + 1 < bytes.len() {
                j += 2;
                continue;
            }
            if c == b'"' {
                in_str = false;
            }
        } else {
            match c {
                b'"' => in_str = true,
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(j);
                    }
                }
                _ => {}
            }
        }
        j += 1;
    }
    None
}

fn cmp_op(t: &Tok<'_>) -> Option<CmpOp> {
    Some(match t {
        Tok::EqEq => CmpOp::Eq,
        Tok::NotEq => CmpOp::Ne,
        Tok::Lt => CmpOp::Lt,
        Tok::Le => CmpOp::Le,
        Tok::Gt => CmpOp::Gt,
        Tok::Ge => CmpOp::Ge,
        _ => return None,
    })
}

fn add_op(t: &Tok<'_>) -> Option<BinOp> {
    Some(match t {
        Tok::Plus => BinOp::Add,
        Tok::Minus => BinOp::Sub,
        _ => return None,
    })
}

fn mul_op(t: &Tok<'_>) -> Option<BinOp> {
    Some(match t {
        Tok::Star => BinOp::Mul,
        Tok::Slash => BinOp::Div,
        Tok::Percent => BinOp::Mod,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    //! `tests/queries.rs` covers the happy path (any query that
    //! compiles has gone through the parser). The unit tests here
    //! pin the error surface: which shapes must be rejected.

    use super::*;
    use crate::lex::tokenize;

    fn rejects(src: &str) {
        let toks = tokenize(src).unwrap_or_default();
        assert!(parse(&toks).is_err(), "expected parse error on `{src}`");
    }

    #[test]
    fn rejects_malformed() {
        rejects(". garbage");
        rejects(". |");
        rejects("[1, 2,");
        rejects("{missing: }");
        rejects("if true then else end");
    }
}
