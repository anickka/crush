use crate::commands::CompileContext;
use crate::errors::JobResult;
use crate::{
    data::Argument,
    stream::{OutputStream, InputStream},
    data::Cell,
    errors::{JobError, argument_error},
};

pub fn get_line_count(arguments: &Vec<Argument>) -> Result<i128, JobError> {
    return match arguments.len() {
        0 => Ok(10),
        1 => match arguments[0].cell {
            Cell::Integer(v) => Ok(v),
            _ => Err(argument_error("Expected a number"))
        }
        _ => Err(argument_error("Too many arguments"))
    };
}

pub fn run(
    lines: i128,
    input: InputStream,
    output: OutputStream,
) -> JobResult<()> {
    let mut count = 0;
    loop {
        match input.recv() {
            Ok(row) => {
                if count >= lines {
                    break;
                }
                output.send(row)?;
                count += 1;
            }
            Err(_) => break,
        }
    }
    return Ok(());
}

pub fn compile_and_run(context: CompileContext) -> JobResult<()> {
    let lines = get_line_count(&context.arguments)?;
    let input = context.input.initialize_stream()?;
    let output = context.output.initialize(input.get_type().clone())?;
    run(lines, input, output)
}
