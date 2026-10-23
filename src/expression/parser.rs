#![allow(clippy::while_let_loop)]

use super::token::Token;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum CompareOp {
    Gt,
    Lt,
    Gte,
    Lte,
    Eq,
}

#[derive(Debug, Clone)]
pub(crate) enum Expr {
    Number(f64),
    Register(String),
    StringLit(String),
    JsonPath(Vec<String>),
    Func(String, Vec<Expr>),
    BinOp {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    UnaryMinus(Box<Expr>),
    Compare {
        op: CompareOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    LogicalAnd(Box<Expr>, Box<Expr>),
    LogicalOr(Box<Expr>, Box<Expr>),
}

pub(crate) struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub(crate) fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<Token> {
        let tok = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        tok
    }

    pub(crate) fn parse(&mut self) -> Result<Expr, String> {
        let expr = self.parse_or()?;
        if self.pos < self.tokens.len() {
            return Err(format!("Unexpected token at position {}", self.pos));
        }
        Ok(expr)
    }

    // or -> and ( '||' and )*
    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_and()?;
        loop {
            match self.peek() {
                Some(Token::Or) => {
                    self.advance();
                    let right = self.parse_and()?;
                    left = Expr::LogicalOr(Box::new(left), Box::new(right));
                }
                _ => break,
            }
        }
        Ok(left)
    }

    // and -> comparison ( '&&' comparison )*
    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_comparison()?;
        loop {
            match self.peek() {
                Some(Token::And) => {
                    self.advance();
                    let right = self.parse_comparison()?;
                    left = Expr::LogicalAnd(Box::new(left), Box::new(right));
                }
                _ => break,
            }
        }
        Ok(left)
    }

    // comparison -> expr ( ('>' | '<' | '>=' | '<=' | '==') expr )*
    fn parse_comparison(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_expr()?;
        loop {
            match self.peek() {
                Some(Token::Gt) | Some(Token::Lt) | Some(Token::Gte) | Some(Token::Lte)
                | Some(Token::Eq) => {
                    let op = match self.advance().unwrap() {
                        Token::Gt => CompareOp::Gt,
                        Token::Lt => CompareOp::Lt,
                        Token::Gte => CompareOp::Gte,
                        Token::Lte => CompareOp::Lte,
                        Token::Eq => CompareOp::Eq,
                        _ => unreachable!(),
                    };
                    let right = self.parse_expr()?;
                    left = Expr::Compare {
                        op,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                _ => break,
            }
        }
        Ok(left)
    }

    // expr -> term (('+' | '-') term)*
    fn parse_expr(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_term()?;
        loop {
            match self.peek() {
                Some(Token::Plus) | Some(Token::Minus) => {
                    let op = match self.advance().unwrap() {
                        Token::Plus => BinOp::Add,
                        Token::Minus => BinOp::Sub,
                        _ => unreachable!(),
                    };
                    let right = self.parse_term()?;
                    left = Expr::BinOp {
                        op,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                _ => break,
            }
        }
        Ok(left)
    }

    // term -> unary (('*' | '/') unary)*
    fn parse_term(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;
        loop {
            match self.peek() {
                Some(Token::Star) | Some(Token::Slash) => {
                    let op = match self.advance().unwrap() {
                        Token::Star => BinOp::Mul,
                        Token::Slash => BinOp::Div,
                        _ => unreachable!(),
                    };
                    let right = self.parse_unary()?;
                    left = Expr::BinOp {
                        op,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                _ => break,
            }
        }
        Ok(left)
    }

    // unary -> '-' unary | atom
    fn parse_unary(&mut self) -> Result<Expr, String> {
        if matches!(self.peek(), Some(Token::Minus)) {
            self.advance();
            let expr = self.parse_unary()?;
            return Ok(Expr::UnaryMinus(Box::new(expr)));
        }
        self.parse_atom()
    }

    // atom -> Number | Register | StringLit | Ident ('.' Ident)* | Ident '(' args ')' | '(' or ')'
    fn parse_atom(&mut self) -> Result<Expr, String> {
        match self.peek().cloned() {
            Some(Token::Number(n)) => {
                self.advance();
                Ok(Expr::Number(n))
            }
            Some(Token::Register(name)) => {
                self.advance();
                Ok(Expr::Register(name))
            }
            Some(Token::StringLit(s)) => {
                self.advance();
                Ok(Expr::StringLit(s))
            }
            Some(Token::Func(name)) => {
                self.advance();
                self.expect_lparen(&name)?;
                let args = self.parse_args()?;
                Ok(Expr::Func(name, args))
            }
            Some(Token::Ident(name)) => {
                self.advance();
                // Check for JSON path: ident ('.' ident)+
                if matches!(self.peek(), Some(Token::Dot)) {
                    let mut path = vec![name];
                    while matches!(self.peek(), Some(Token::Dot)) {
                        self.advance();
                        match self.advance() {
                            Some(Token::Ident(part)) | Some(Token::Func(part)) => {
                                path.push(part);
                            }
                            _ => {
                                return Err(format!(
                                    "Expected identifier after '.' in JSON path at position {}",
                                    self.pos
                                ));
                            }
                        }
                    }
                    Ok(Expr::JsonPath(path))
                } else if matches!(self.peek(), Some(Token::LParen)) {
                    // Function call
                    self.advance();
                    let args = self.parse_args()?;
                    Ok(Expr::Func(name, args))
                } else {
                    // Just an identifier - treat as single-element JSON path for flexibility
                    Ok(Expr::JsonPath(vec![name]))
                }
            }
            Some(Token::LParen) => {
                self.advance();
                let expr = self.parse_or()?;
                match self.advance() {
                    Some(Token::RParen) => Ok(expr),
                    _ => Err("Expected ')'".into()),
                }
            }
            _ => Err(format!("Unexpected token at position {}", self.pos)),
        }
    }

    fn expect_lparen(&mut self, name: &str) -> Result<(), String> {
        match self.advance() {
            Some(Token::LParen) => Ok(()),
            _ => Err(format!("Expected '(' after function '{name}'")),
        }
    }

    fn parse_args(&mut self) -> Result<Vec<Expr>, String> {
        let mut args = Vec::new();
        if matches!(self.peek(), Some(Token::RParen)) {
            self.advance();
            return Ok(args);
        }
        loop {
            args.push(self.parse_or()?);
            match self.advance() {
                Some(Token::Comma) => continue,
                Some(Token::RParen) => break,
                _ => return Err("Expected ',' or ')' in function arguments".into()),
            }
        }
        Ok(args)
    }
}
