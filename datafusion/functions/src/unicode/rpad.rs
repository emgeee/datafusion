// Licensed to the Apache Software Foundation (ASF) under one
// or more contributor license agreements.  See the NOTICE file
// distributed with this work for additional information
// regarding copyright ownership.  The ASF licenses this file
// to you under the Apache License, Version 2.0 (the
// "License"); you may not use this file except in compliance
// with the License.  You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing,
// software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.  See the License for the
// specific language governing permissions and limitations
// under the License.

use std::any::Any;
use std::sync::Arc;

use arrow::array::{ArrayRef, GenericStringArray, OffsetSizeTrait};
use arrow::datatypes::DataType;
use datafusion_common::cast::{
    as_generic_string_array, as_int64_array, as_string_view_array,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::utils::{make_scalar_function, utf8_to_str_type};
use datafusion_common::{exec_err, Result};
use datafusion_expr::TypeSignature::Exact;
use datafusion_expr::{ColumnarValue, ScalarUDFImpl, Signature, Volatility};

#[derive(Debug)]
pub struct RPadFunc {
    signature: Signature,
}

impl Default for RPadFunc {
    fn default() -> Self {
        Self::new()
    }
}

impl RPadFunc {
    pub fn new() -> Self {
        use DataType::*;
        Self {
            signature: Signature::one_of(
                vec![
                    Exact(vec![Utf8View, Int64]),
                    Exact(vec![Utf8View, Int64, Utf8View]),
                    Exact(vec![Utf8View, Int64, Utf8]),
                    Exact(vec![Utf8View, Int64, LargeUtf8]),
                    Exact(vec![Utf8, Int64]),
                    Exact(vec![Utf8, Int64, Utf8View]),
                    Exact(vec![Utf8, Int64, Utf8]),
                    Exact(vec![Utf8, Int64, LargeUtf8]),
                    Exact(vec![LargeUtf8, Int64]),
                    Exact(vec![LargeUtf8, Int64, Utf8View]),
                    Exact(vec![LargeUtf8, Int64, Utf8]),
                    Exact(vec![LargeUtf8, Int64, LargeUtf8]),
                ],
                Volatility::Immutable,
            ),
        }
    }
}

impl ScalarUDFImpl for RPadFunc {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn name(&self) -> &str {
        "rpad"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, arg_types: &[DataType]) -> Result<DataType> {
        utf8_to_str_type(&arg_types[0], "rpad")
    }

    fn invoke(&self, args: &[ColumnarValue]) -> Result<ColumnarValue> {
        match args.len() {
            2 => match args[0].data_type() {
                DataType::Utf8 | DataType::Utf8View => {
                    make_scalar_function(rpad::<i32, i32>, vec![])(args)
                }
                DataType::LargeUtf8 => {
                    make_scalar_function(rpad::<i64, i64>, vec![])(args)
                }
                other => exec_err!("Unsupported data type {other:?} for function rpad"),
            },
            3 => match (args[0].data_type(), args[2].data_type()) {
                (
                    DataType::Utf8 | DataType::Utf8View,
                    DataType::Utf8 | DataType::Utf8View,
                ) => make_scalar_function(rpad::<i32, i32>, vec![])(args),
                (DataType::LargeUtf8, DataType::LargeUtf8) => {
                    make_scalar_function(rpad::<i64, i64>, vec![])(args)
                }
                (DataType::LargeUtf8, DataType::Utf8View | DataType::Utf8) => {
                    make_scalar_function(rpad::<i64, i32>, vec![])(args)
                }
                (DataType::Utf8View | DataType::Utf8, DataType::LargeUtf8) => {
                    make_scalar_function(rpad::<i32, i64>, vec![])(args)
                }
                (first_type, last_type) => {
                    exec_err!("unsupported arguments type for rpad, first argument type is {}, last argument type is {}", first_type, last_type)
                }
            },
            number => {
                exec_err!("unsupported arguments number {} for rpad", number)
            }
        }
    }
}

macro_rules! process_rpad {
    // For the two-argument case
    ($string_array:expr, $length_array:expr) => {{
        $string_array
            .iter()
            .zip($length_array.iter())
            .map(|(string, length)| match (string, length) {
                (Some(string), Some(length)) => {
                    if length > i32::MAX as i64 {
                        return exec_err!("rpad requested length {} too large", length);
                    }

                    let length = if length < 0 { 0 } else { length as usize };
                    if length == 0 {
                        Ok(Some("".to_string()))
                    } else {
                        let graphemes = string.graphemes(true).collect::<Vec<&str>>();
                        if length < graphemes.len() {
                            Ok(Some(graphemes[..length].concat()))
                        } else {
                            let mut s = string.to_string();
                            s.push_str(" ".repeat(length - graphemes.len()).as_str());
                            Ok(Some(s))
                        }
                    }
                }
                _ => Ok(None),
            })
            .collect::<Result<GenericStringArray<StringArrayLen>>>()
    }};

    // For the three-argument case
    ($string_array:expr, $length_array:expr, $fill_array:expr) => {{
        $string_array
            .iter()
            .zip($length_array.iter())
            .zip($fill_array.iter())
            .map(|((string, length), fill)| match (string, length, fill) {
                (Some(string), Some(length), Some(fill)) => {
                    if length > i32::MAX as i64 {
                        return exec_err!("rpad requested length {} too large", length);
                    }

                    let length = if length < 0 { 0 } else { length as usize };
                    let graphemes = string.graphemes(true).collect::<Vec<&str>>();
                    let fill_chars = fill.chars().collect::<Vec<char>>();

                    if length < graphemes.len() {
                        Ok(Some(graphemes[..length].concat()))
                    } else if fill_chars.is_empty() {
                        Ok(Some(string.to_string()))
                    } else {
                        let mut s = string.to_string();
                        let char_vector: Vec<char> = (0..length - graphemes.len())
                            .map(|l| fill_chars[l % fill_chars.len()])
                            .collect();
                        s.push_str(&char_vector.iter().collect::<String>());
                        Ok(Some(s))
                    }
                }
                _ => Ok(None),
            })
            .collect::<Result<GenericStringArray<StringArrayLen>>>()
    }};
}

/// Extends the string to length 'length' by appending the characters fill (a space by default). If the string is already longer than length then it is truncated.
/// rpad('hi', 5, 'xy') = 'hixyx'
pub fn rpad<StringArrayLen: OffsetSizeTrait, FillArrayLen: OffsetSizeTrait>(
    args: &[ArrayRef],
) -> Result<ArrayRef> {
    match (args.len(), args[0].data_type()) {
        (2, DataType::Utf8View) => {
            let string_array = as_string_view_array(&args[0])?;
            let length_array = as_int64_array(&args[1])?;

            let result = process_rpad!(string_array, length_array)?;
            Ok(Arc::new(result) as ArrayRef)
        }
        (2, _) => {
            let string_array = as_generic_string_array::<StringArrayLen>(&args[0])?;
            let length_array = as_int64_array(&args[1])?;

            let result = process_rpad!(string_array, length_array)?;
            Ok(Arc::new(result) as ArrayRef)
        }
        (3, DataType::Utf8View) => {
            let string_array = as_string_view_array(&args[0])?;
            let length_array = as_int64_array(&args[1])?;
            match args[2].data_type() {
                DataType::Utf8View => {
                    let fill_array = as_string_view_array(&args[2])?;
                    let result = process_rpad!(string_array, length_array, fill_array)?;
                    Ok(Arc::new(result) as ArrayRef)
                }
                DataType::Utf8 | DataType::LargeUtf8 => {
                    let fill_array = as_generic_string_array::<FillArrayLen>(&args[2])?;
                    let result = process_rpad!(string_array, length_array, fill_array)?;
                    Ok(Arc::new(result) as ArrayRef)
                }
                other_type => {
                    exec_err!("unsupported type for rpad's third operator: {}", other_type)
                }
            }
        }
        (3, _) => {
            let string_array = as_generic_string_array::<StringArrayLen>(&args[0])?;
            let length_array = as_int64_array(&args[1])?;
            match args[2].data_type() {
                DataType::Utf8View => {
                    let fill_array = as_string_view_array(&args[2])?;
                    let result = process_rpad!(string_array, length_array, fill_array)?;
                    Ok(Arc::new(result) as ArrayRef)
                }
                DataType::Utf8 | DataType::LargeUtf8 => {
                    let fill_array = as_generic_string_array::<FillArrayLen>(&args[2])?;
                    let result = process_rpad!(string_array, length_array, fill_array)?;
                    Ok(Arc::new(result) as ArrayRef)
                }
                other_type => {
                    exec_err!("unsupported type for rpad's third operator: {}", other_type)
                }
            }
        }
        (other, other_type) => exec_err!(
            "rpad requires 2 or 3 arguments with corresponding types, but got {}. number of arguments with {}",
            other, other_type
        ),
    }
}

#[cfg(test)]
mod tests {
    use arrow::array::{Array, StringArray};
    use arrow::datatypes::DataType::Utf8;

    use datafusion_common::{Result, ScalarValue};
    use datafusion_expr::{ColumnarValue, ScalarUDFImpl};

    use crate::unicode::rpad::RPadFunc;
    use crate::utils::test::test_function;

    #[test]
    fn test_functions() -> Result<()> {
        test_function!(
            RPadFunc::new(),
            &[
                ColumnarValue::Scalar(ScalarValue::from("josé")),
                ColumnarValue::Scalar(ScalarValue::from(5i64)),
            ],
            Ok(Some("josé ")),
            &str,
            Utf8,
            StringArray
        );
        test_function!(
            RPadFunc::new(),
            &[
                ColumnarValue::Scalar(ScalarValue::from("hi")),
                ColumnarValue::Scalar(ScalarValue::from(5i64)),
            ],
            Ok(Some("hi   ")),
            &str,
            Utf8,
            StringArray
        );
        test_function!(
            RPadFunc::new(),
            &[
                ColumnarValue::Scalar(ScalarValue::from("hi")),
                ColumnarValue::Scalar(ScalarValue::from(0i64)),
            ],
            Ok(Some("")),
            &str,
            Utf8,
            StringArray
        );
        test_function!(
            RPadFunc::new(),
            &[
                ColumnarValue::Scalar(ScalarValue::from("hi")),
                ColumnarValue::Scalar(ScalarValue::Int64(None)),
            ],
            Ok(None),
            &str,
            Utf8,
            StringArray
        );
        test_function!(
            RPadFunc::new(),
            &[
                ColumnarValue::Scalar(ScalarValue::Utf8(None)),
                ColumnarValue::Scalar(ScalarValue::from(5i64)),
            ],
            Ok(None),
            &str,
            Utf8,
            StringArray
        );
        test_function!(
            RPadFunc::new(),
            &[
                ColumnarValue::Scalar(ScalarValue::from("hi")),
                ColumnarValue::Scalar(ScalarValue::from(5i64)),
                ColumnarValue::Scalar(ScalarValue::from("xy")),
            ],
            Ok(Some("hixyx")),
            &str,
            Utf8,
            StringArray
        );
        test_function!(
            RPadFunc::new(),
            &[
                ColumnarValue::Scalar(ScalarValue::from("hi")),
                ColumnarValue::Scalar(ScalarValue::from(21i64)),
                ColumnarValue::Scalar(ScalarValue::from("abcdef")),
            ],
            Ok(Some("hiabcdefabcdefabcdefa")),
            &str,
            Utf8,
            StringArray
        );
        test_function!(
            RPadFunc::new(),
            &[
                ColumnarValue::Scalar(ScalarValue::from("hi")),
                ColumnarValue::Scalar(ScalarValue::from(5i64)),
                ColumnarValue::Scalar(ScalarValue::from(" ")),
            ],
            Ok(Some("hi   ")),
            &str,
            Utf8,
            StringArray
        );
        test_function!(
            RPadFunc::new(),
            &[
                ColumnarValue::Scalar(ScalarValue::from("hi")),
                ColumnarValue::Scalar(ScalarValue::from(5i64)),
                ColumnarValue::Scalar(ScalarValue::from("")),
            ],
            Ok(Some("hi")),
            &str,
            Utf8,
            StringArray
        );
        test_function!(
            RPadFunc::new(),
            &[
                ColumnarValue::Scalar(ScalarValue::Utf8(None)),
                ColumnarValue::Scalar(ScalarValue::from(5i64)),
                ColumnarValue::Scalar(ScalarValue::from("xy")),
            ],
            Ok(None),
            &str,
            Utf8,
            StringArray
        );
        test_function!(
            RPadFunc::new(),
            &[
                ColumnarValue::Scalar(ScalarValue::from("hi")),
                ColumnarValue::Scalar(ScalarValue::Int64(None)),
                ColumnarValue::Scalar(ScalarValue::from("xy")),
            ],
            Ok(None),
            &str,
            Utf8,
            StringArray
        );
        test_function!(
            RPadFunc::new(),
            &[
                ColumnarValue::Scalar(ScalarValue::from("hi")),
                ColumnarValue::Scalar(ScalarValue::from(5i64)),
                ColumnarValue::Scalar(ScalarValue::Utf8(None)),
            ],
            Ok(None),
            &str,
            Utf8,
            StringArray
        );
        test_function!(
            RPadFunc::new(),
            &[
                ColumnarValue::Scalar(ScalarValue::from("josé")),
                ColumnarValue::Scalar(ScalarValue::from(10i64)),
                ColumnarValue::Scalar(ScalarValue::from("xy")),
            ],
            Ok(Some("joséxyxyxy")),
            &str,
            Utf8,
            StringArray
        );
        test_function!(
            RPadFunc::new(),
            &[
                ColumnarValue::Scalar(ScalarValue::from("josé")),
                ColumnarValue::Scalar(ScalarValue::from(10i64)),
                ColumnarValue::Scalar(ScalarValue::from("éñ")),
            ],
            Ok(Some("josééñéñéñ")),
            &str,
            Utf8,
            StringArray
        );
        #[cfg(not(feature = "unicode_expressions"))]
        test_function!(
            RPadFunc::new(),
            &[
                ColumnarValue::Scalar(ScalarValue::from("josé")),
                ColumnarValue::Scalar(ScalarValue::from(5i64)),
            ],
            internal_err!(
                "function rpad requires compilation with feature flag: unicode_expressions."
            ),
            &str,
            Utf8,
            StringArray
        );

        Ok(())
    }
}
