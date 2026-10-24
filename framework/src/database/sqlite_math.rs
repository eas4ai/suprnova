//! Supply geo math on each SQLite connection when its build lacks native math.

use std::ffi::{CStr, c_int};

use libsqlite3_sys as ffi;
use sea_orm::sqlx::{self, SqliteConnection};

struct MathFunction {
    name: &'static CStr,
    arity: c_int,
    evaluate: fn(f64, f64) -> f64,
}

static FUNCTIONS: [MathFunction; 6] = [
    MathFunction {
        name: c"radians",
        arity: 1,
        evaluate: |x, _| x.to_radians(),
    },
    MathFunction {
        name: c"sin",
        arity: 1,
        evaluate: |x, _| x.sin(),
    },
    MathFunction {
        name: c"cos",
        arity: 1,
        evaluate: |x, _| x.cos(),
    },
    MathFunction {
        name: c"asin",
        arity: 1,
        evaluate: |x, _| x.asin(),
    },
    MathFunction {
        name: c"sqrt",
        arity: 1,
        evaluate: |x, _| x.sqrt(),
    },
    MathFunction {
        name: c"power",
        arity: 2,
        evaluate: f64::powf,
    },
];

pub(super) async fn ensure(connection: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM pragma_function_list WHERE name IN ('radians', 'sin', 'cos', 'asin', 'sqrt', 'power')",
    ).fetch_all(&mut *connection).await?;
    if FUNCTIONS.iter().all(|function| {
        names
            .iter()
            .any(|name| name.as_bytes() == function.name.to_bytes())
    }) {
        return Ok(());
    }
    let mut handle = connection.lock_handle().await?;
    for function in &FUNCTIONS {
        if names
            .iter()
            .any(|name| name.as_bytes() == function.name.to_bytes())
        {
            continue;
        }
        // SAFETY: SQLx's guard holds exclusive access to the live connection.
        // User data points to a static descriptor, and SQLite calls `evaluate`
        // with exactly the registered arity. No destructor is needed for static data.
        let result = unsafe {
            ffi::sqlite3_create_function_v2(
                handle.as_raw_handle().as_ptr(),
                function.name.as_ptr(),
                function.arity,
                ffi::SQLITE_UTF8 | ffi::SQLITE_DETERMINISTIC | ffi::SQLITE_INNOCUOUS,
                std::ptr::from_ref(function).cast_mut().cast(),
                Some(evaluate),
                None,
                None,
                None,
            )
        };
        if result != ffi::SQLITE_OK {
            return Err(sqlx::Error::Protocol(format!(
                "could not register SQLite geo math function {}: code {result}",
                function.name.to_string_lossy()
            )));
        }
    }
    Ok(())
}

unsafe extern "C" fn evaluate(
    context: *mut ffi::sqlite3_context,
    argc: c_int,
    argv: *mut *mut ffi::sqlite3_value,
) {
    // SAFETY: only `ensure` registers this callback. SQLite supplies a live
    // context, the static descriptor and `argc` valid value pointers for the
    // duration of this call. All operations below stay within that lifetime.
    unsafe {
        let function = &*ffi::sqlite3_user_data(context).cast::<MathFunction>();
        if argc != function.arity || !(1..=2).contains(&argc) {
            ffi::sqlite3_result_null(context);
            return;
        }
        let arguments = std::slice::from_raw_parts(argv, argc as usize);
        let mut values = [0.0; 2];
        for (index, argument) in arguments.iter().enumerate() {
            if ffi::sqlite3_value_type(*argument) == ffi::SQLITE_NULL {
                ffi::sqlite3_result_null(context);
                return;
            }
            values[index] = ffi::sqlite3_value_double(*argument);
        }
        let value = (function.evaluate)(values[0], values[1]);
        if value.is_finite() {
            ffi::sqlite3_result_double(context, value);
        } else {
            // SQLite's native math also returns NULL for domain errors.
            ffi::sqlite3_result_null(context);
        }
    }
}
