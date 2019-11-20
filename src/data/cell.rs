use std::cmp::Ordering;
use std::hash::Hasher;
use std::path::Path;

use chrono::{DateTime, Local};
use regex::Regex;

use crate::{
    closure::Closure,
    env::get_cwd,
    data::rows::Rows,
    errors::{error, JobError, to_job_error},
    glob::Glob,
};
use crate::data::{List, Command, Output, CellType, Dict, ColumnType};
use crate::errors::JobResult;
use std::time::Duration;
use crate::format::duration_format;
use crate::env::Env;
use crate::data::row::RowWithTypes;
use crate::stream::streams;

#[derive(Debug)]
pub enum Cell {
    Text(Box<str>),
    Integer(i128),
    Time(DateTime<Local>),
    Duration(Duration),
    Field(Vec<Box<str>>),
    Glob(Glob),
    Regex(Box<str>, Regex),
    Op(Box<str>),
    Command(Command),
    Closure(Closure),
    Output(Output),
    File(Box<Path>),
    Rows(Rows),
    Row(RowWithTypes),
    List(List),
    Dict(Dict),
    Env(Env),
    Bool(bool),
}

impl Cell {
    pub fn to_string(&self) -> String {
        return match self {
            Cell::Text(val) => val.to_string(),
            Cell::Integer(val) => val.to_string(),
            Cell::Time(val) => val.format("%Y-%m-%d %H:%M:%S %z").to_string(),
            Cell::Field(val) => format!(r"%{}", val.join(".")),
            Cell::Glob(val) => format!("*{{{}}}", val.to_string()),
            Cell::Regex(val, _) => format!("r{{{}}}", val),
            Cell::Op(val) => val.to_string(),
            Cell::Command(_) => "Command".to_string(),
            Cell::File(val) => val.to_str().unwrap_or("<Broken file>").to_string(),
            Cell::Rows(_) => "<Rows>".to_string(),
            Cell::Row(row) => row.to_string(),
            Cell::Closure(c) => c.to_string(),
            Cell::Output(_) => "<Output>".to_string(),
            Cell::List(l) => l.to_string(),
            Cell::Duration(d) => duration_format(d),
            Cell::Env(env) => env.to_string(),
            Cell::Bool(v) => (if *v { "true" } else { "false" }).to_string(),
            Cell::Dict(d) => d.to_string(),
        };
    }

    pub fn alignment(&self) -> Alignment {
        return match self {
            Cell::Time(_) | Cell::Duration(_) | Cell::Integer(_) => Alignment::Right,
            _ => Alignment::Left,
        };
    }

    pub fn empty_stream() -> Cell {
        let (s, r) = streams(vec![]);
        Cell::Output(Output {stream: r})
    }

    pub fn text(s: &str) -> Cell {
        Cell::Text(Box::from(s))
    }

    pub fn op(s: &str) -> Cell {
        Cell::Op(Box::from(s))
    }

    pub fn cell_type(&self) -> CellType {
        return match self {
            Cell::Text(_) => CellType::Text,
            Cell::Integer(_) => CellType::Integer,
            Cell::Time(_) => CellType::Time,
            Cell::Field(_) => CellType::Field,
            Cell::Glob(_) => CellType::Glob,
            Cell::Regex(_, _) => CellType::Regex,
            Cell::Op(_) => CellType::Op,
            Cell::Command(_) => CellType::Command,
            Cell::File(_) => CellType::File,
            Cell::Output(o) => CellType::Output(o.stream.get_type().clone()),
            Cell::Rows(r) => CellType::Rows(r.types.clone()),
            Cell::Row(r) => CellType::Row(r.types.clone()),
            Cell::Closure(_) => CellType::Closure,
            Cell::List(l) => l.list_type(),
            Cell::Duration(_) => CellType::Duration,
            Cell::Env(_) => CellType::Env,
            Cell::Bool(_) => CellType::Bool,
            Cell::Dict(d) => d.dict_type(),
        };
    }

    pub fn file_expand(&self, v: &mut Vec<Box<Path>>) -> JobResult<()> {
        match self {
            Cell::Text(s) => v.push(Box::from(Path::new(s.as_ref()))),
            Cell::File(p) => v.push(p.clone()),
            Cell::Glob(pattern) => to_job_error(pattern.glob_files(
                &get_cwd()?, v))?,
            _ => return Err(error("Expected a file name")),
        }
        Ok(())
    }

    pub fn partial_clone(&self) -> Result<Cell, JobError> {
        return match self {
            Cell::Text(v) => Ok(Cell::Text(v.clone())),
            Cell::Integer(v) => Ok(Cell::Integer(v.clone())),
            Cell::Time(v) => Ok(Cell::Time(v.clone())),
            Cell::Field(v) => Ok(Cell::Field(v.clone())),
            Cell::Glob(v) => Ok(Cell::Glob(v.clone())),
            Cell::Regex(v, r) => Ok(Cell::Regex(v.clone(), r.clone())),
            Cell::Op(v) => Ok(Cell::Op(v.clone())),
            Cell::Command(v) => Ok(Cell::Command(v.clone())),
            Cell::File(v) => Ok(Cell::File(v.clone())),
            Cell::Rows(r) => Ok(Cell::Rows(r.partial_clone()?)),
            Cell::Row(r) => Ok(Cell::Row(r.partial_clone()?)),
            Cell::Closure(c) => Ok(Cell::Closure(c.clone())),
            Cell::Output(_) => Err(error("Invalid use of stream")),
            Cell::List(l) => Ok(Cell::List(l.partial_clone()?)),
            Cell::Duration(d) => Ok(Cell::Duration(d.clone())),
            Cell::Env(e) => Ok(Cell::Env(e.clone())),
            Cell::Bool(v) => Ok(Cell::Bool(v.clone())),
            Cell::Dict(d) => Ok(Cell::Dict(d.partial_clone()?))
        };
    }

    pub fn materialize(self) -> Cell {
        match self {
            Cell::Output(output) => {
                let mut rows = Vec::new();
                loop {
                    match output.stream.recv() {
                        Ok(r) => rows.push(r.materialize()),
                        Err(_) => break,
                    }
                }
                Cell::Rows(Rows { types: ColumnType::materialize(output.stream.get_type()), rows: rows })
            }
            Cell::Rows(r) => Cell::Rows(r.materialize()),
            Cell::Dict(d) => Cell::Dict(d.materialize()),
            Cell::Row(r) => Cell::Row(r.materialize()),
            Cell::List(l) => Cell::List(l.materialize()),
            _ => self,
        }
    }

    pub fn cast(self, new_type: CellType) -> Result<Cell, JobError> {
        if self.cell_type() == new_type {
            return Ok(self);
        }

        /*
        This function is silly and overly large. Instead of mathcing on every source/destination pair, it should do
        two matches, one to convert any cell to a string, and one to convert a string to any cell. That would shorten
        this monstrosity to a sane size.
        */
        match (self, new_type) {
            (Cell::Text(s), CellType::File) => Ok(Cell::File(Box::from(Path::new(s.as_ref())))),
            (Cell::Text(s), CellType::Glob) => Ok(Cell::Glob(Glob::new(&s))),
            (Cell::Text(s), CellType::Integer) => to_job_error(s.parse::<i128>()).map(|v| Cell::Integer(v)),
            (Cell::Text(s), CellType::Field) => Ok(Cell::Field(vec![s])),
            (Cell::Text(s), CellType::Op) => Ok(Cell::Op(s)),
            (Cell::Text(s), CellType::Regex) => to_job_error(Regex::new(s.as_ref()).map(|v| Cell::Regex(s, v))),

            (Cell::File(s), CellType::Text) => match s.to_str() {
                Some(s) => Ok(Cell::Text(Box::from(s))),
                None => Err(error("File name is not valid unicode"))
            },
            (Cell::File(s), CellType::Glob) => match s.to_str() {
                Some(s) => Ok(Cell::Glob(Glob::new(s))),
                None => Err(error("File name is not valid unicode"))
            },
            (Cell::File(s), CellType::Integer) => match s.to_str() {
                Some(s) => to_job_error(s.parse::<i128>()).map(|v| Cell::Integer(v)),
                None => Err(error("File name is not valid unicode"))
            },
            (Cell::File(s), CellType::Op) => match s.to_str() {
                Some(s) => Ok(Cell::Op(Box::from(s))),
                None => Err(error("File name is not valid unicode"))
            },
            (Cell::File(s), CellType::Regex) => match s.to_str() {
                Some(s) => to_job_error(Regex::new(s.as_ref()).map(|v| Cell::Regex(Box::from(s), v))),
                None => Err(error("File name is not valid unicode"))
            },

            (Cell::Glob(s), CellType::Text) => Ok(Cell::Text(s.to_string().clone().into_boxed_str())),
            (Cell::Glob(s), CellType::File) => Ok(Cell::File(Box::from(Path::new(s.to_string().as_str())))),
            (Cell::Glob(s), CellType::Integer) => to_job_error(s.to_string().parse::<i128>()).map(|v| Cell::Integer(v)),
            (Cell::Glob(s), CellType::Op) => Ok(Cell::op(s.to_string().as_str())),
            (Cell::Glob(g), CellType::Regex) => {
                let s = g.to_string().as_str();
                to_job_error(Regex::new(s).map(|v| Cell::Regex(Box::from(s), v)))
            }
            /*
                        (Cell::Field(s), CellType::File) => Ok(Cell::File(Box::from(Path::new(s.as_ref())))),
                        (Cell::Field(s), CellType::Glob) => Ok(Cell::Glob(Glob::new(&s))),
                        (Cell::Field(s), CellType::Integer) => to_job_error(s.parse::<i128>()).map(|v| Cell::Integer(v)),
                        (Cell::Field(s), CellType::Text) => Ok(Cell::Text(s)),
                        (Cell::Field(s), CellType::Op) => Ok(Cell::Op(s)),
                        (Cell::Field(s), CellType::Regex) => to_job_error(Regex::new(s.as_ref()).map(|v| Cell::Regex(s, v))),
            */
            (Cell::Regex(s, _), CellType::File) => Ok(Cell::File(Box::from(Path::new(s.as_ref())))),
            (Cell::Regex(s, _), CellType::Glob) => Ok(Cell::Glob(Glob::new(&s))),
            (Cell::Regex(s, _), CellType::Integer) => to_job_error(s.parse::<i128>()).map(|v| Cell::Integer(v)),
            (Cell::Regex(s, _), CellType::Text) => Ok(Cell::Text(s)),
            (Cell::Regex(s, _), CellType::Op) => Ok(Cell::Op(s)),

            (Cell::Integer(i), CellType::Text) => Ok(Cell::Text(i.to_string().into_boxed_str())),
            (Cell::Integer(i), CellType::File) => Ok(Cell::File(Box::from(Path::new(i.to_string().as_str())))),
            (Cell::Integer(i), CellType::Glob) => Ok(Cell::Glob(Glob::new(i.to_string().as_str()))),
            (Cell::Integer(i), CellType::Field) => Ok(Cell::Field(vec![i.to_string().into_boxed_str()])),
            (Cell::Integer(i), CellType::Op) => Ok(Cell::Op(i.to_string().into_boxed_str())),
            (Cell::Integer(i), CellType::Regex) => {
                let s = i.to_string();
                to_job_error(Regex::new(s.as_str()).map(|v| Cell::Regex(s.into_boxed_str(), v)))
            }
            _ => Err(error("Unimplemented conversion")),
        }
    }
}

impl std::hash::Hash for Cell {
    fn hash<H: Hasher>(&self, state: &mut H) {
        if !self.cell_type().is_hashable() {
            panic!("Can't hash mutable cell types!");
        }
        match self {
            Cell::Text(v) => v.hash(state),
            Cell::Integer(v) => v.hash(state),
            Cell::Time(v) => v.hash(state),
            Cell::Field(v) => v.hash(state),
            Cell::Glob(v) => v.hash(state),
            Cell::Regex(v, _) => v.hash(state),
            Cell::Op(v) => v.hash(state),
            Cell::Command(_) => {}
            Cell::File(v) => v.hash(state),
            Cell::Duration(d) => d.hash(state),
            Cell::Bool(v) => v.hash(state),

            Cell::Env(_) | Cell::Dict(_) | Cell::Rows(_) | Cell::Closure(_) |
            Cell::List(_) | Cell::Output(_) | Cell::Row(_) => panic!("Can't hash output"),
        }
    }
}

fn file_result_compare(f1: &Path, f2: &Path) -> bool {
    match (f1.canonicalize(), f2.canonicalize()) {
        (Ok(p1), Ok(p2)) => p1 == p2,
        _ => false,
    }
}

impl std::cmp::PartialEq for Cell {
    fn eq(&self, other: &Cell) -> bool {
        return match (self, other) {
            (Cell::Text(val1), Cell::Text(val2)) => val1 == val2,
            (Cell::Glob(glb), Cell::Text(val)) => glb.matches(val),
            (Cell::Text(val), Cell::Glob(glb)) => glb.matches(val),
            (Cell::Integer(val1), Cell::Integer(val2)) => val1 == val2,
            (Cell::Time(val1), Cell::Time(val2)) => val1 == val2,
            (Cell::Duration(val1), Cell::Duration(val2)) => val1 == val2,
            (Cell::Field(val1), Cell::Field(val2)) => val1 == val2,
            (Cell::Glob(val1), Cell::Glob(val2)) => val1 == val2,
            (Cell::Regex(val1, _), Cell::Regex(val2, _)) => val1 == val2,
            (Cell::Op(val1), Cell::Op(val2)) => val1 == val2,
            (Cell::Command(val1), Cell::Command(val2)) => val1 == val2,
            (Cell::List(val1), Cell::List(val2)) => val1 == val2,
            (Cell::Rows(val1), Cell::Rows(val2)) => match val1.partial_cmp(val2) {
                None => false,
                Some(o) => o == Ordering::Equal,
            },
            (Cell::Row(val1), Cell::Row(val2)) => match val1.partial_cmp(val2) {
                None => false,
                Some(o) => o == Ordering::Equal,
            },
            (Cell::File(val1), Cell::File(val2)) => file_result_compare(val1.as_ref(), val2.as_ref()),
            (Cell::Text(val1), Cell::File(val2)) => file_result_compare(&Path::new(&val1.to_string()), val2.as_ref()),
            (Cell::File(val1), Cell::Text(val2)) => file_result_compare(&Path::new(&val2.to_string()), val1.as_ref()),
            (Cell::Bool(val1), Cell::Bool(val2)) => val1 == val2,
            _ => false,
        };
    }
}

pub enum Alignment {
    Left,
    Right,
}

impl std::cmp::PartialOrd for Cell {
    fn partial_cmp(&self, other: &Cell) -> Option<Ordering> {
        let t1 = self.cell_type();
        let t2 = other.cell_type();
        if t1 != t2 {
            return Some(t1.cmp(&t2));
        }
        return match (self, other) {
            (Cell::Text(val1), Cell::Text(val2)) => Some(val1.cmp(val2)),
            (Cell::Field(val1), Cell::Field(val2)) => Some(val1.cmp(val2)),
            (Cell::Glob(val1), Cell::Glob(val2)) => Some(val1.cmp(val2)),
            (Cell::Regex(val1, _), Cell::Regex(val2, _)) => Some(val1.cmp(val2)),
            (Cell::Integer(val1), Cell::Integer(val2)) => Some(val1.cmp(val2)),
            (Cell::Time(val1), Cell::Time(val2)) => Some(val1.cmp(val2)),
            (Cell::Op(val1), Cell::Op(val2)) => Some(val1.cmp(val2)),
            (Cell::File(val1), Cell::File(val2)) => Some(val1.cmp(val2)),
            (Cell::Duration(val1), Cell::Duration(val2)) => Some(val1.cmp(val2)),
            (Cell::Command(_), Cell::Command(_)) => None,
            (Cell::Closure(_), _) => None,
            (Cell::Output(_), _) => None,
            (Cell::Rows(val1), Cell::Rows(val2)) => val1.partial_cmp(val2),
            (Cell::Row(val1), Cell::Row(val2)) => val1.partial_cmp(val2),
            (Cell::List(val1), Cell::List(val2)) => val1.partial_cmp(val2),
            (Cell::Bool(val1), Cell::Bool(val2)) => Some(val1.cmp(val2)),
            _ => None,
        };
    }
}

impl std::cmp::Eq for Cell {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_casts() {
        assert_eq!(Cell::Text(Box::from("112432")).cast(CellType::Integer).is_err(), false);
        assert_eq!(Cell::text("1d").cast(CellType::Integer).is_err(), true);
        assert_eq!(Cell::text("1d").cast(CellType::Glob).is_err(), false);
        assert_eq!(Cell::text("1d").cast(CellType::File).is_err(), false);
        assert_eq!(Cell::text("1d").cast(CellType::Time).is_err(), true);
        assert_eq!(Cell::text("fad").cast(CellType::Field).is_err(), false);
        assert_eq!(Cell::text("fad").cast(CellType::Op).is_err(), false);
    }

    #[test]
    fn test_duration_format() {
        assert_eq!(duration_format(&Duration::from_micros(0)), "0".to_string());
        assert_eq!(duration_format(&Duration::from_micros(1)), "0.000001".to_string());
        assert_eq!(duration_format(&Duration::from_micros(100)), "0.0001".to_string());
        assert_eq!(duration_format(&Duration::from_millis(1)), "0.001".to_string());
        assert_eq!(duration_format(&Duration::from_millis(1000)), "1".to_string());
        assert_eq!(duration_format(&Duration::from_millis(1000 * 61)), "1:01".to_string());
        assert_eq!(duration_format(&Duration::from_millis(1000 * 3601)), "1:00:01".to_string());
        assert_eq!(duration_format(&Duration::from_millis(1000 * (3600 * 24 * 3 + 1))), "3d0:00:01".to_string());
        assert_eq!(duration_format(&Duration::from_millis(1000 * (3600 * 24 * 365 * 10 + 1))), "10y0d0:00:01".to_string());
        assert_eq!(duration_format(&Duration::from_millis(1000 * (3600 * 24 * 365 * 10 + 1) + 1)), "10y0d0:00:01".to_string());
    }
}
