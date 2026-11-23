use magick_script::{
    Operation,
    plan::{ExecutionPlan, ExecutionStep},
};

use zosimos::command;

pub struct MagickPlanner {
    execution_plan: ExecutionPlan,
}

pub enum TranslationError {
    Unsupported,
}

impl MagickPlanner {
    pub fn to_command(&self) -> Result<command::CommandBuffer, TranslationError> {
        let mut command_buffer = command::CommandBuffer::default();

        for step in self.execution_plan.execution() {
            match step {
                ExecutionStep::InputFile(file) => {
                    let var = command_buffer.generic(command::GenericDeclaration::empty());
                    let file_in = command_buffer.input_generic(var).unwrap();

                    for op in &file.ops {
                        todo!()
                    }
                }
                ExecutionStep::Map(operation) => {
                    todo!()
                },
                ExecutionStep::Rewrite(rewrite_operation) => {
                    todo!()
                },
                ExecutionStep::Write(location, file_format) => {
                    todo!()
                }
            }
        }

        Ok(command_buffer)
    }
}

impl From<ExecutionPlan> for MagickPlanner {
    fn from(execution_plan: ExecutionPlan) -> Self {
        MagickPlanner { execution_plan }
    }
}
