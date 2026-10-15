//! Compile Python-shaped row predicates once; never execute Python code.
use super::{b2f, truthy, Tok};
use crate::{
    common::no_unit,
    dataframe::{ColumnData, DataFrame},
    error::{DvError, Result},
};
use std::borrow::Cow;

fn error(message: &str) -> DvError {
    DvError::Formula(message.into())
}

enum Value<'a> {
    Variable,
    Text(&'a str),
    Number(f64),
    Stringify(Box<Value<'a>>),
}
enum RowValue<'a> {
    Text(Cow<'a, str>),
    Number(f64),
}
impl Value<'_> {
    fn evaluate<'a>(&'a self, column: &'a ColumnData, row: usize) -> RowValue<'a> {
        match self {
            Self::Variable => match column {
                ColumnData::Str(values) => RowValue::Text(Cow::Borrowed(&values[row])),
                ColumnData::Float(values) | ColumnData::DateTime(values) => {
                    RowValue::Number(values[row])
                }
            },
            Self::Text(text) => RowValue::Text(Cow::Borrowed(text)),
            Self::Number(number) => RowValue::Number(*number),
            Self::Stringify(value) => match value.evaluate(column, row) {
                RowValue::Text(text) => RowValue::Text(text),
                RowValue::Number(number) => RowValue::Text(Cow::Owned(if number.is_nan() {
                    "nan".into()
                } else if number == f64::INFINITY {
                    "inf".into()
                } else if number == f64::NEG_INFINITY {
                    "-inf".into()
                } else {
                    format!("{number:?}")
                })),
            },
        }
    }
}
enum Predicate<'a> {
    Value(Value<'a>),
    Compare(Value<'a>, &'static str, Value<'a>),
    And(Box<Predicate<'a>>, Box<Predicate<'a>>),
    Or(Box<Predicate<'a>>, Box<Predicate<'a>>),
    Not(Box<Predicate<'a>>),
}
impl Predicate<'_> {
    fn evaluate(&self, column: &ColumnData, row: usize) -> Result<bool> {
        Ok(match self {
            Self::Value(value) => match value.evaluate(column, row) {
                RowValue::Text(text) => !text.is_empty(),
                RowValue::Number(number) => truthy(number),
            },
            Self::And(a, b) => a.evaluate(column, row)? && b.evaluate(column, row)?,
            Self::Or(a, b) => a.evaluate(column, row)? || b.evaluate(column, row)?,
            Self::Not(value) => !value.evaluate(column, row)?,
            Self::Compare(a, op, b) => match (a.evaluate(column, row), b.evaluate(column, row)) {
                (RowValue::Text(a), RowValue::Text(b)) => match *op {
                    "in" => b.contains(a.as_ref()),
                    "not in" => !b.contains(a.as_ref()),
                    "==" => a == b,
                    "!=" => a != b,
                    "<" => a < b,
                    "<=" => a <= b,
                    ">" => a > b,
                    ">=" => a >= b,
                    _ => unreachable!(),
                },
                (RowValue::Number(a), RowValue::Number(b)) => match *op {
                    "==" => a == b,
                    "!=" => a != b,
                    "<" => a < b,
                    "<=" => a <= b,
                    ">" => a > b,
                    ">=" => a >= b,
                    _ => return Err(error("membership requires strings")),
                },
                _ => return Err(error("cannot compare text and numeric values")),
            },
        })
    }
}
struct Parser<'a> {
    tokens: &'a [Tok],
    pos: usize,
    variable: &'a str,
}
impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&'a Tok> {
        self.tokens.get(self.pos)
    }
    fn keyword(&mut self, name: &str) -> bool {
        if matches!(self.peek(),Some(Tok::Ident(word)) if word==name) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn operator(&mut self, operators: &[&str]) -> Option<&'static str> {
        match self.peek() {
            Some(Tok::Op(op)) if operators.contains(op) => {
                self.pos += 1;
                Some(*op)
            }
            _ => None,
        }
    }
    fn or(&mut self) -> Result<Predicate<'a>> {
        let mut value = self.and()?;
        while self.operator(&["|", "||"]).is_some() || self.keyword("or") {
            value = Predicate::Or(Box::new(value), Box::new(self.and()?));
        }
        Ok(value)
    }
    fn and(&mut self) -> Result<Predicate<'a>> {
        let mut value = self.not()?;
        while self.operator(&["&", "&&"]).is_some() || self.keyword("and") {
            value = Predicate::And(Box::new(value), Box::new(self.not()?));
        }
        Ok(value)
    }
    fn not(&mut self) -> Result<Predicate<'a>> {
        if self.operator(&["~"]).is_some() || self.keyword("not") {
            return Ok(Predicate::Not(Box::new(self.not()?)));
        }
        if self.peek() == Some(&Tok::LParen) {
            self.pos += 1;
            let value = self.or()?;
            if self.peek() != Some(&Tok::RParen) {
                return Err(error("expected closing predicate parenthesis"));
            }
            self.pos += 1;
            return Ok(value);
        }
        let left = self.value()?;
        let op = if let Some(op) = self.operator(&["==", "!=", "<", "<=", ">", ">="]) {
            Some(op)
        } else if self.keyword("in") {
            Some("in")
        } else if self.keyword("not") {
            if !self.keyword("in") {
                return Err(error("expected `in` after `not`"));
            }
            Some("not in")
        } else {
            None
        };
        Ok(if let Some(op) = op {
            Predicate::Compare(left, op, self.value()?)
        } else {
            Predicate::Value(left)
        })
    }
    fn value(&mut self) -> Result<Value<'a>> {
        let token = self
            .peek()
            .ok_or_else(|| error("missing row-predicate operand"))?;
        self.pos += 1;
        match token {
            Tok::Str(text)=>Ok(Value::Text(text)),Tok::Num(number)=>Ok(Value::Number(*number)),
            Tok::Ident(name) if name==self.variable=>Ok(Value::Variable),
            Tok::Ident(name) if name=="True"=>Ok(Value::Number(1.)),Tok::Ident(name) if name=="False"=>Ok(Value::Number(0.)),
            Tok::Op("-")=>match self.value()?{Value::Number(number)=>Ok(Value::Number(-number)),_=>Err(error("unary minus requires a numeric operand"))},
            Tok::Ident(name) if name=="str"=>{
                if self.peek()!=Some(&Tok::LParen){return Err(error("expected `str(value)`"));}
                self.pos+=1;let value=self.value()?;
                if self.peek()!=Some(&Tok::RParen){return Err(error("expected closing `str` parenthesis"));}
                self.pos+=1;Ok(Value::Stringify(Box::new(value)))
            },
            _=>Err(error("unsupported row predicate; use literals, its bound variable, comparisons, membership, str() and boolean operators")),
        }
    }
}
pub(super) fn evaluate(df: &DataFrame, tokens: &[Tok]) -> Result<Vec<f64>> {
    if tokens.len() < 7 {
        return Err(error("expected `[predicate for variable in {Column}]`"));
    }
    let suffix = tokens.len() - 5;
    let (variable, name) = match &tokens[suffix..] {
        [Tok::Ident(keyword), Tok::Ident(variable), Tok::Ident(membership), Tok::Col(name), Tok::RBracket]
            if keyword == "for" && membership == "in" =>
        {
            (variable.as_str(), name.as_str())
        }
        _ => return Err(error("expected `[predicate for variable in {Column}]`")),
    };
    let mut parser = Parser {
        tokens: &tokens[1..suffix],
        pos: 0,
        variable,
    };
    let predicate = parser.or()?;
    if parser.pos != parser.tokens.len() {
        return Err(error("unexpected trailing row-predicate token"));
    }
    let column = df
        .columns()
        .iter()
        .find(|c| no_unit(&c.name).trim() == name)
        .or_else(|| df.columns().iter().find(|c| c.name.trim() == name))
        .ok_or_else(|| DvError::Formula(format!("unknown column `{{{name}}}`")))?;
    (0..df.nrows())
        .map(|row| predicate.evaluate(&column.data, row).map(b2f))
        .collect()
}
