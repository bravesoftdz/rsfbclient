//!
//! Rust Firebird Client
//!
//! Status of connetions, transactions...
//!

use std::fmt::Write;
use thiserror::Error;

use crate::{ibase, row::ColumnType};

pub struct Status(Box<ibase::ISC_STATUS_ARRAY>);

impl Default for Status {
    fn default() -> Self {
        Status(Box::new([0; 20]))
    }
}

impl Status {
    pub fn sql_code(&self, ibase: &ibase::IBase) -> i32 {
        unsafe { ibase.isc_sqlcode()(self.0.as_ptr()) }
    }

    pub fn message(&self, ibase: &ibase::IBase) -> String {
        let mut buffer: Vec<u8> = Vec::with_capacity(256);
        let mut msg = String::new();

        let mut ptr = self.0.as_ptr();

        loop {
            unsafe {
                let len = ibase.fb_interpret()(
                    buffer.as_mut_ptr() as *mut _,
                    buffer.capacity() as u32,
                    &mut ptr,
                );
                buffer.set_len(len as usize);
            }

            if buffer.is_empty() {
                break;
            }

            writeln!(
                &mut msg,
                "{}",
                std::str::from_utf8(&buffer).unwrap_or("Invalid error message")
            )
            .unwrap();
        }
        // Remove the last \n
        msg.pop();

        msg
    }

    pub fn as_error(&self, ibase: &ibase::IBase) -> FbError {
        FbError::Sql(SqlError {
            code: self.sql_code(ibase),
            msg: self.message(ibase),
        })
    }

    pub fn as_mut_ptr(&mut self) -> *mut isize {
        self.0.as_mut_ptr()
    }
}

#[derive(Debug, Error)]
pub enum FbError {
    #[error("sql error {}: {}", .0.code, .0.msg)]
    Sql(SqlError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("error: {0}")]
    Other(String),
}

impl From<String> for FbError {
    fn from(msg: String) -> Self {
        Self::Other(msg)
    }
}

impl From<&str> for FbError {
    fn from(msg: &str) -> Self {
        Self::Other(msg.to_string())
    }
}

#[derive(Debug)]
pub struct SqlError {
    pub msg: String,
    pub code: i32,
}

pub fn err_idx_not_exist<T>() -> Result<T, FbError> {
    Err(FbError::Other("This index doesn't exists".to_string()))
}

pub fn err_column_null<T>(type_name: &str) -> Result<T, FbError> {
    Err(FbError::Other(format!(
        "This is a null value. Use the Option<{}> to safe access this column and avoid errors",
        type_name
    )))
}

pub fn err_type_conv<T>(from: ColumnType, to: &str) -> Result<T, FbError> {
    Err(FbError::Other(format!(
        "Can't convert {:?} column to {}",
        from, to
    )))
}

pub fn err_buffer_len<T>(expected: usize, found: usize, type_name: &str) -> Result<T, FbError> {
    Err(FbError::Other(format!(
        "Invalid buffer size for type {:?} (expected: {}, found: {})",
        type_name, expected, found
    )))
}
