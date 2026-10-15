//! Inert pickle parser: records constructors and restore states, never invokes them.
use dv_core::IoError;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};
#[derive(Debug, Clone)]
pub(super) enum Value {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
    Seq(Vec<Node>),
    Dict(Vec<(Node, Node)>),
    Global(String, String),
    Object {
        constructor: Node,
        args: Node,
        state: Option<Node>,
    },
}
pub(super) type Node = Rc<RefCell<Value>>;
fn node(v: Value) -> Node {
    Rc::new(RefCell::new(v))
}
struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    stack: Vec<Node>,
    marks: Vec<usize>,
    memo: HashMap<usize, Node>,
}
impl<'a> Parser<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], IoError> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| super::broken("pickle length overflow"))?;
        let b = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| super::broken("truncated pickle"))?;
        self.pos = end;
        Ok(b)
    }
    fn byte(&mut self) -> Result<u8, IoError> {
        Ok(self.bytes(1)?[0])
    }
    fn line(&mut self) -> Result<String, IoError> {
        let n = self.bytes[self.pos..]
            .iter()
            .position(|&b| b == b'\n')
            .ok_or_else(|| super::broken("unterminated pickle line"))?;
        let b = self.bytes(n)?;
        self.byte()?;
        String::from_utf8(b.to_vec()).map_err(super::broken)
    }
    fn uint(&mut self, n: usize) -> Result<usize, IoError> {
        let b = self.bytes(n)?;
        let mut v = 0u64;
        for (i, &b) in b.iter().enumerate() {
            v |= (b as u64) << (8 * i);
        }
        usize::try_from(v).map_err(super::broken)
    }
    fn pop(&mut self) -> Result<Node, IoError> {
        self.stack
            .pop()
            .ok_or_else(|| super::broken("pickle stack underflow"))
    }
    fn top(&self) -> Result<Node, IoError> {
        self.stack
            .last()
            .cloned()
            .ok_or_else(|| super::broken("pickle stack underflow"))
    }
    fn marked(&mut self) -> Result<Vec<Node>, IoError> {
        let mark = self
            .marks
            .pop()
            .ok_or_else(|| super::broken("pickle missing MARK"))?;
        if mark > self.stack.len() {
            return Err(super::broken("pickle MARK points beyond stack"));
        }
        Ok(self.stack.split_off(mark))
    }
    fn push(&mut self, v: Value) {
        self.stack.push(node(v));
    }
    fn seq(&mut self, n: usize) -> Result<(), IoError> {
        let i = self
            .stack
            .len()
            .checked_sub(n)
            .ok_or_else(|| super::broken("pickle tuple underflow"))?;
        let v = self.stack.split_off(i);
        self.push(Value::Seq(v));
        Ok(())
    }
    fn pairs(v: Vec<Node>) -> Result<Vec<(Node, Node)>, IoError> {
        if v.len() % 2 != 0 {
            return Err(super::broken("pickle dictionary missing value"));
        }
        Ok(v.chunks(2).map(|v| (v[0].clone(), v[1].clone())).collect())
    }
    fn memo_get(&mut self, i: usize) -> Result<(), IoError> {
        let v = self
            .memo
            .get(&i)
            .cloned()
            .ok_or_else(|| super::broken("pickle missing memo entry"))?;
        self.stack.push(v);
        Ok(())
    }
    fn decode(mut self) -> Result<Node, IoError> {
        loop {
            let op = self.byte()?;
            match op {
                b'.' => return self.pop(),
                0x80 => { let p = self.byte()?; if p > 5 { return Err(IoError::Other(format!("unsupported pickle protocol {p}"))); } },
                0x95 => { let _ = self.uint(8)?; },
                b'(' => self.marks.push(self.stack.len()), b'0' => { if self.marks.last()==Some(&self.stack.len()){self.marks.pop();}else{self.pop()?;} }, b'1' => { self.marked()?; }, b'2' => { let v = self.top()?; self.stack.push(v); },
                b'N' => self.push(Value::None), 0x88 => self.push(Value::Bool(true)), 0x89 => self.push(Value::Bool(false)),
                b'K' => { let v = self.byte()?; self.push(Value::Int(v as i64)); },
                b'M' => { let v = self.uint(2)?; self.push(Value::Int(v as i64)); },
                b'J' => { let b: [u8;4] = self.bytes(4)?.try_into().unwrap(); self.push(Value::Int(i32::from_le_bytes(b) as i64)); },
                b'G' => { let b: [u8;8] = self.bytes(8)?.try_into().unwrap(); self.push(Value::Float(f64::from_be_bytes(b))); },
                b'F' => { let s = self.line()?; self.push(Value::Float(s.parse().map_err(super::broken)?)); },
                b'I' | b'L' => { let s = self.line()?; let s = s.trim_end_matches('L'); let v = if s == "01" { Value::Bool(true) } else if s == "00" { Value::Bool(false) } else { Value::Int(s.parse().map_err(super::broken)?) }; self.push(v); },
                0x8a | 0x8b => {
                    let n = if op == 0x8a { self.uint(1)? } else { self.uint(4)? }; let b = self.bytes(n)?;
                    if n > 8 { return Err(IoError::Other("pickle integer exceeds dv-core numeric range".into())); }
                    let sign = b.last().is_some_and(|&b| b & 0x80 != 0); let mut raw = if sign { [255;8] } else { [0;8] }; raw[..n].copy_from_slice(b); self.push(Value::Int(i64::from_le_bytes(raw)));
                },
                b'X' | 0x8c | 0x8d => { let n = self.uint(if op == 0x8c {1} else if op == b'X' {4} else {8})?; let s = String::from_utf8(self.bytes(n)?.to_vec()).map_err(super::broken)?; self.push(Value::Text(s)); },
                b'B' | b'C' | 0x8e | 0x96 | b'T' | b'U' => { let n = self.uint(if op == b'C' || op == b'U' {1} else if op == b'B' || op == b'T' {4} else {8})?; let b = self.bytes(n)?.to_vec(); self.push(Value::Bytes(b)); },
                b'V' => { let s = self.line()?; self.push(Value::Text(s)); },
                b'S' => return Err(IoError::Other("protocol-0 quoted string opcodes are not supported; use binary pickle protocols 1-5".into())),
                b']' | b')' => self.push(Value::Seq(Vec::new())), b'l' | b't' => { let v = self.marked()?; self.push(Value::Seq(v)); },
                0x85 => self.seq(1)?, 0x86 => self.seq(2)?, 0x87 => self.seq(3)?,
                b'a' => { let v = self.pop()?; let top = self.top()?; if let Value::Seq(s) = &mut *top.borrow_mut() { s.push(v); } else { return Err(super::broken("pickle APPEND on non-list")); }; },
                b'e' => { let v = self.marked()?; let top = self.top()?; if let Value::Seq(s) = &mut *top.borrow_mut() { s.extend(v); } else { return Err(super::broken("pickle APPENDS on non-list")); }; },
                b'}' => self.push(Value::Dict(Vec::new())), b'd' => { let v = Self::pairs(self.marked()?)?; self.push(Value::Dict(v)); },
                b's' => { let v = self.pop()?; let k = self.pop()?; let top = self.top()?; if let Value::Dict(d) = &mut *top.borrow_mut() { d.push((k,v)); } else { return Err(super::broken("pickle SETITEM on non-dict")); }; },
                b'u' => { let v = Self::pairs(self.marked()?)?; let top = self.top()?; if let Value::Dict(d) = &mut *top.borrow_mut() { d.extend(v); } else { return Err(super::broken("pickle SETITEMS on non-dict")); }; },
                b'q' | b'r' => { let i = self.uint(if op == b'q' {1} else {4})?; self.memo.insert(i,self.top()?); },
                b'p' => { let i = self.line()?.parse().map_err(super::broken)?; self.memo.insert(i,self.top()?); },
                0x94 => { let i = self.memo.len(); self.memo.insert(i,self.top()?); },
                b'h' | b'j' => { let i = self.uint(if op == b'h' {1} else {4})?; self.memo_get(i)?; }, b'g' => { let i = self.line()?.parse().map_err(super::broken)?; self.memo_get(i)?; },
                b'c' => { let module = self.line()?; let name = self.line()?; self.push(Value::Global(module,name)); },
                0x93 => { let name = text(&self.pop()?)?; let module = text(&self.pop()?)?; self.push(Value::Global(module,name)); },
                b'R' | 0x81 => { let args = self.pop()?; let constructor = self.pop()?; self.push(Value::Object {constructor,args,state:None}); },
                0x92 => return Err(IoError::Other("pickle keyword-argument object construction is not supported".into())),
                b'b' => { let state = self.pop()?; let top = self.top()?; if let Value::Object {state:s,..} = &mut *top.borrow_mut() { *s = Some(state); } else { return Err(super::broken("pickle BUILD on non-object")); }; },
                _ => return Err(IoError::Other(format!("unsupported pickle opcode 0x{op:02x}; persistent IDs, recursive graphs, extensions and out-of-band buffers are not supported"))),
            }
        }
    }
}
pub(super) fn text(n: &Node) -> Result<String, IoError> {
    match &*n.borrow() {
        Value::Text(s) => Ok(s.clone()),
        Value::Bytes(b) => String::from_utf8(b.clone()).map_err(super::broken),
        Value::Int(i) => Ok(i.to_string()),
        _ => Err(super::broken("pickle value is not a string")),
    }
}
pub(super) fn sequence(n: &Node) -> Result<Vec<Node>, IoError> {
    match &*n.borrow() {
        Value::Seq(s) => Ok(s.clone()),
        _ => Err(super::broken("pickle value is not a sequence")),
    }
}
pub(super) fn integer(n: &Node) -> Result<usize, IoError> {
    match &*n.borrow() {
        Value::Int(v) => usize::try_from(*v).map_err(super::broken),
        _ => Err(super::broken("pickle shape is not an integer")),
    }
}
pub(super) fn parse(bytes: &[u8]) -> Result<Node, IoError> {
    let root = Parser {
        bytes,
        pos: 0,
        stack: Vec::new(),
        marks: Vec::new(),
        memo: HashMap::new(),
    }
    .decode()?;
    let mut pending = vec![(root.clone(), false)];
    let mut active = HashSet::new();
    let mut finished = HashSet::new();
    while let Some((n, exit)) = pending.pop() {
        let id = Rc::as_ptr(&n);
        if exit {
            active.remove(&id);
            finished.insert(id);
            continue;
        }
        if finished.contains(&id) {
            continue;
        }
        if !active.insert(id) {
            return Err(super::broken("recursive pickle graphs are not supported"));
        }
        pending.push((n.clone(), true));
        match &*n.borrow() {
            Value::Seq(v) => pending.extend(v.iter().rev().map(|n| (n.clone(), false))),
            Value::Dict(v) => {
                for (k, v) in v.iter().rev() {
                    pending.push((v.clone(), false));
                    pending.push((k.clone(), false));
                }
            }
            Value::Object {
                constructor,
                args,
                state,
            } => {
                if let Some(s) = state {
                    pending.push((s.clone(), false));
                }
                pending.push((args.clone(), false));
                pending.push((constructor.clone(), false));
            }
            _ => {}
        };
    }
    Ok(root)
}
