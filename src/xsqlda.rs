#![allow(non_upper_case_globals)]

use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::{convert::TryFrom, mem};

use crate::{
    ibase::{self, consts},
    row::ColumnType,
    FbError,
};

/// Data for the statement to return
pub const XSQLDA_DESCRIBE_VARS: [u8; 18] = [
    ibase::isc_info_sql_stmt_type as u8, // Statement type: StmtType
    ibase::isc_info_sql_select as u8,    // Select columns
    ibase::isc_info_sql_describe_vars as u8, // Column count
    ibase::isc_info_sql_sqlda_seq as u8, // Column index
    ibase::isc_info_sql_type as u8,      // Sql Type code
    ibase::isc_info_sql_sub_type as u8,  // Blob subtype
    ibase::isc_info_sql_scale as u8,     // Decimal / Numeric scale
    ibase::isc_info_sql_length as u8,    // Data length
    ibase::isc_info_sql_null_ind as u8,  // Null indicator (0 or -1)
    ibase::isc_info_sql_field as u8,     //
    ibase::isc_info_sql_relation as u8,  //
    ibase::isc_info_sql_owner as u8,     //
    ibase::isc_info_sql_alias as u8,     // Column alias
    ibase::isc_info_sql_describe_end as u8, // End of column data
    ibase::isc_info_sql_bind as u8,      // Select params
    ibase::isc_info_sql_describe_vars as u8, // Param count
    ibase::isc_info_sql_sqlda_seq as u8, // Param index
    ibase::isc_info_sql_describe_end as u8, // End of param data
];

#[derive(Debug, Default)]
/// Sql query column information
pub struct XSqlVar {
    /// Sql type code
    pub sqltype: i16,

    /// Scale: indicates that the real value is `data * 10.pow(scale)`
    pub scale: i16,

    /// Blob subtype code
    pub sqlsubtype: i16,

    /// Length of the column data
    pub data_length: i16,

    /// Null indicator
    pub null_ind: bool,

    pub field_name: String,

    pub relation_name: String,

    pub owner_name: String,

    /// Column alias
    pub alias_name: String,
}

impl XSqlVar {
    /// Coerces the data types of this XSqlVar as necessary
    pub fn coerce(&mut self) -> Result<(), FbError> {
        // Remove nullable type indicator
        let sqltype = self.sqltype & (!1);

        // var.null_ind = 1;

        match sqltype as u32 {
            ibase::SQL_TEXT | ibase::SQL_VARYING => {
                self.sqltype = ColumnType::Text as i16 + 1;
            }

            ibase::SQL_SHORT | ibase::SQL_LONG | ibase::SQL_INT64 => {
                self.data_length = mem::size_of::<i64>() as i16;

                if self.scale == 0 {
                    self.sqltype = ColumnType::Integer as i16 + 1;
                } else {
                    // Is actually a decimal or numeric value, so coerce as double
                    self.scale = 0;
                    self.sqltype = ColumnType::Float as i16 + 1;
                }
            }

            ibase::SQL_FLOAT | ibase::SQL_DOUBLE => {
                self.data_length = mem::size_of::<i64>() as i16;

                self.sqltype = ColumnType::Float as i16 + 1;
            }

            ibase::SQL_TIMESTAMP | ibase::SQL_TYPE_DATE | ibase::SQL_TYPE_TIME => {
                self.data_length = mem::size_of::<ibase::ISC_TIMESTAMP>() as i16;

                self.sqltype = ColumnType::Timestamp as i16 + 1;
            }

            sqltype => {
                return Err(format!("Unsupported column type ({})", sqltype).into());
            }
        }

        Ok(())
    }

    pub fn to_column_type(&self) -> Result<ColumnType, FbError> {
        // Remove nullable type indicator
        let sqltype = self.sqltype & (!1);

        ColumnType::try_from(sqltype as u32).map_err(|_| {
            FbError::Other(format!(
                "Conversion from sql type {} not implemented",
                sqltype
            ))
        })
    }
}

/// Convert the xsqlda a blr (binary representation)
pub fn xsqlda_to_blr(xsqlda: &[XSqlVar]) -> Result<Bytes, FbError> {
    let mut blr = BytesMut::with_capacity(256);
    blr.put_slice(&[
        consts::blr::VERSION5,
        consts::blr::BEGIN,
        consts::blr::MESSAGE,
        0, // Message index
    ]);
    // Message length, * 2 as there is 1 msg for the param type and another for the nullind
    blr.put_u16_le(xsqlda.len() as u16 * 2);

    for var in xsqlda {
        let column_type = var.to_column_type()?;

        match column_type {
            ColumnType::Text => {
                blr.put_u8(consts::blr::VARYING);
                blr.put_i16_le(var.data_length);
            }

            ColumnType::Integer => blr.put_slice(&[
                consts::blr::INT64,
                0, // Scale
            ]),

            ColumnType::Float => blr.put_u8(consts::blr::DOUBLE),

            ColumnType::Timestamp => blr.put_u8(consts::blr::TIMESTAMP),
        }
        // Nullind
        blr.put_slice(&[consts::blr::SHORT, 0]);
    }

    blr.put_slice(&[consts::blr::END, consts::blr::EOC]);

    Ok(blr.freeze())
}

/// Data returned for a prepare statement
pub struct PrepareInfo {
    pub stmt_type: ibase::StmtType,
    pub xsqlda: Vec<XSqlVar>,
    pub param_count: usize,
    pub truncated: bool,
}

/// Parses the data from the `PrepareStatement` response.
///
/// XSqlDa data format: u8 type + optional data preceded by a u16 length.
/// Returns the statement type, xsqlda and an indicator if the data was truncated (xsqlda not entirely filled)
pub fn parse_xsqlda(resp: &mut Bytes) -> Result<PrepareInfo, FbError> {
    // Asserts that the first
    if resp.remaining() < 7 || resp[..3] != [ibase::isc_info_sql_stmt_type as u8, 0x04, 0x00] {
        return err_invalid_xsqlda();
    }
    resp.advance(3);

    let stmt_type =
        ibase::StmtType::try_from(resp.get_u32_le()).map_err(|e| FbError::Other(e.to_string()))?;

    let mut xsqlda = Vec::new();

    let (truncated, param_count) = if resp.remaining() >= 4
        && resp.bytes()[..2]
            == [
                ibase::isc_info_sql_select as u8,
                ibase::isc_info_sql_describe_vars as u8,
            ] {
        resp.advance(2);

        let len = resp.get_u16_le() as usize;
        if resp.remaining() < len {
            return err_invalid_xsqlda();
        }

        let col_len = resp.get_uint_le(len) as usize;
        xsqlda = Vec::with_capacity(col_len);

        parse_select_items(resp, &mut xsqlda)?
    } else {
        (false, 0)
    };

    Ok(PrepareInfo {
        stmt_type,
        xsqlda,
        param_count,
        truncated,
    })
}

/// Fill the xsqlda with data from the cursor, return `true` if the data was truncated (needs more data to fill the xsqlda)
/// and the number of parameters
fn parse_select_items(
    resp: &mut Bytes,
    xsqlda: &mut Vec<XSqlVar>,
) -> Result<(bool, usize), FbError> {
    if resp.remaining() == 0 {
        return Ok((false, 0));
    }

    /// What data are we reading?
    #[derive(Debug, Copy, Clone)]
    enum Reading {
        Column,
        Param,
    };
    let mut kind = Reading::Column;

    let mut col_index = 0;
    let mut param_index = 0;
    let mut param_count = 0;

    let truncated = loop {
        if resp.remaining() == 0 {
            return err_invalid_xsqlda();
        }
        // Get item code
        match resp.get_u8() as u32 {
            // Column data to follow
            ibase::isc_info_sql_select => kind = Reading::Column,

            // Parameter data to follow
            ibase::isc_info_sql_bind => kind = Reading::Param,

            // Start describing the columns / params
            ibase::isc_info_sql_describe_vars => {
                if resp.remaining() < 6 {
                    return err_invalid_xsqlda();
                }
                // Assume 0x04 0x00
                resp.advance(2);
                let count = resp.get_u32_le();

                if let Reading::Param = kind {
                    param_count = count;
                }
            }

            // Column / Param index
            ibase::isc_info_sql_sqlda_seq => {
                if resp.remaining() < 6 {
                    return err_invalid_xsqlda();
                }
                // Assume 0x04 0x00
                resp.advance(2);

                match kind {
                    Reading::Column => {
                        // Index received starts on 1
                        col_index = resp.get_u32_le() as usize - 1;

                        xsqlda.push(Default::default());
                        // Must be the same
                        debug_assert_eq!(xsqlda.len() - 1, col_index);
                    }
                    Reading::Param => {
                        // Index received starts on 1
                        param_index = resp.get_u32_le() as usize - 1;
                    }
                }
            }

            ibase::isc_info_sql_type => {
                if resp.remaining() < 6 {
                    return err_invalid_xsqlda();
                }
                // Assume 0x04 0x00
                resp.advance(2);

                if let Some(var) = xsqlda.get_mut(col_index) {
                    var.sqltype = resp.get_i32_le() as i16;
                } else {
                    return err_invalid_xsqlda();
                }
            }

            ibase::isc_info_sql_sub_type => {
                if resp.remaining() < 6 {
                    return err_invalid_xsqlda();
                }
                // Assume 0x04 0x00
                resp.advance(2);

                if let Some(var) = xsqlda.get_mut(col_index) {
                    var.sqlsubtype = resp.get_i32_le() as i16;
                } else {
                    return err_invalid_xsqlda();
                }
            }

            ibase::isc_info_sql_scale => {
                if resp.remaining() < 6 {
                    return err_invalid_xsqlda();
                }
                // Assume 0x04 0x00
                resp.advance(2);

                if let Some(var) = xsqlda.get_mut(col_index) {
                    var.scale = resp.get_i32_le() as i16;
                } else {
                    return err_invalid_xsqlda();
                }
            }

            ibase::isc_info_sql_length => {
                if resp.remaining() < 6 {
                    return err_invalid_xsqlda();
                }
                // Assume 0x04 0x00
                resp.advance(2);

                if let Some(var) = xsqlda.get_mut(col_index) {
                    var.data_length = resp.get_i32_le() as i16;
                } else {
                    return err_invalid_xsqlda();
                }
            }

            ibase::isc_info_sql_null_ind => {
                if resp.remaining() < 6 {
                    return err_invalid_xsqlda();
                }
                // Assume 0x04 0x00
                resp.advance(2);

                if let Some(var) = xsqlda.get_mut(col_index) {
                    var.null_ind = resp.get_i32_le() != 0;
                } else {
                    return err_invalid_xsqlda();
                }
            }

            ibase::isc_info_sql_field => {
                if resp.remaining() < 2 {
                    return err_invalid_xsqlda();
                }
                let len = resp.get_u16_le() as usize;

                if resp.remaining() < len {
                    return err_invalid_xsqlda();
                }
                let mut buff = vec![0; len];
                resp.copy_to_slice(&mut buff);

                if let Some(var) = xsqlda.get_mut(col_index) {
                    var.field_name = String::from_utf8(buff).unwrap_or_default();
                } else {
                    return err_invalid_xsqlda();
                }
            }

            ibase::isc_info_sql_relation => {
                if resp.remaining() < 2 {
                    return err_invalid_xsqlda();
                }
                let len = resp.get_u16_le() as usize;

                if resp.remaining() < len {
                    return err_invalid_xsqlda();
                }
                let mut buff = vec![0; len];
                resp.copy_to_slice(&mut buff);

                if let Some(var) = xsqlda.get_mut(col_index) {
                    var.relation_name = String::from_utf8(buff).unwrap_or_default();
                } else {
                    return err_invalid_xsqlda();
                }
            }

            ibase::isc_info_sql_owner => {
                if resp.remaining() < 2 {
                    return err_invalid_xsqlda();
                }
                let len = resp.get_u16_le() as usize;

                if resp.remaining() < len {
                    return err_invalid_xsqlda();
                }
                let mut buff = vec![0; len];
                resp.copy_to_slice(&mut buff);

                if let Some(var) = xsqlda.get_mut(col_index) {
                    var.owner_name = String::from_utf8(buff).unwrap_or_default();
                } else {
                    return err_invalid_xsqlda();
                }
            }

            ibase::isc_info_sql_alias => {
                if resp.remaining() < 2 {
                    return err_invalid_xsqlda();
                }
                let len = resp.get_u16_le() as usize;

                if resp.remaining() < len {
                    return err_invalid_xsqlda();
                }
                let mut buff = vec![0; len];
                resp.copy_to_slice(&mut buff);

                if let Some(var) = xsqlda.get_mut(col_index) {
                    var.alias_name = String::from_utf8(buff).unwrap_or_default();
                } else {
                    return err_invalid_xsqlda();
                }
            }

            // Data truncated
            ibase::isc_info_truncated => break true,

            // End of this column / param
            ibase::isc_info_sql_describe_end => {}

            // End of the data
            ibase::isc_info_end => break false,

            item => {
                return Err(FbError::Other(format!(
                    "Invalid item received in the xsqlda: {}",
                    item
                )));
            }
        }
    };

    Ok((truncated, param_count as usize))
}

fn err_invalid_xsqlda<T>() -> Result<T, FbError> {
    Err(FbError::Other(
        "Invalid Xsqlda received from server".to_string(),
    ))
}
