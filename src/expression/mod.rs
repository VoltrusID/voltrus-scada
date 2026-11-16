mod dsl;
mod evaluator;
mod parser;
#[cfg(test)]
mod tests;
mod token;

pub use dsl::run_dsl;
pub use evaluator::{check_syntax, evaluate, evaluate_json};
