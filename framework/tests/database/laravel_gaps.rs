//! Contracts for transaction scopes and the model-less query builder.

use crate::query_fixture::Fixture;
use sea_orm::DbBackend;
use std::sync::{Arc, Mutex};
use suprnova::{DB, FrameworkError, UpdateAttrs, attrs};

async fn fixture() -> Fixture {
    let fx = Fixture::sqlite().await;
    fx.exec(
        "CREATE TABLE gap_rows (id INTEGER PRIMARY KEY, score INTEGER, grp TEXT, created_at TEXT)",
    )
    .await;
    fx.exec("INSERT INTO gap_rows VALUES (1, 10, 'a', '2026-10-03'), (2, 20, 'a', '2026-10-01'), (3, 30, 'b', '2026-10-02')").await;
    fx
}

async fn ids(query: suprnova::DbTableBuilder) -> Vec<i64> {
    query
        .order_by_asc("id")
        .get()
        .await
        .unwrap()
        .iter()
        .map(|row| row.get_int("id").unwrap())
        .collect()
}

#[tokio::test]
async fn missing_table_rows_report_default_and_custom_404_messages() {
    let _fx = fixture().await;
    let query = || DB::table("gap_rows").filter("id", 99);
    let error = query().first_or_fail().await.unwrap_err();
    assert_eq!(error.status_code(), 404);
    assert!(error.to_string().contains("gap_rows"));
    let error = query()
        .first_or_fail_with("Choose another row")
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), 404);
    assert!(error.to_string().contains("Choose another row"));
    assert_eq!(
        DB::table("gap_rows")
            .first_or_fail_with("unused")
            .await
            .unwrap()
            .get_int("id")
            .unwrap(),
        1
    );
    assert_eq!(
        DB::table("missing_table")
            .first_or_fail()
            .await
            .unwrap_err()
            .status_code(),
        500
    );
}

#[tokio::test]
async fn inner_rollback_preserves_outer_rows_depth_and_callback_ownership() {
    let _fx = fixture().await;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let sink = calls.clone();
    assert_eq!(DB::transaction_level(), 0);
    DB::transaction(move |_tx| {
        Box::pin(async move {
            assert_eq!(DB::transaction_level(), 1);
            DB::table("gap_rows")
                .filter("id", 1)
                .update(attrs! { score: 11 })
                .await?;
            let outer = sink.clone();
            DB::after_commit(move || async move {
                outer.lock().unwrap().push("outer commit");
                Ok(())
            })
            .await?;
            let outer = sink.clone();
            DB::after_rollback(move || async move {
                outer.lock().unwrap().push("outer rollback");
                Ok(())
            })
            .await?;
            let inner = sink.clone();
            let failed = DB::transaction(move |_tx| {
                Box::pin(async move {
                    assert_eq!(DB::transaction_level(), 2);
                    DB::table("gap_rows")
                        .filter("id", 2)
                        .update(attrs! { score: 22 })
                        .await?;
                    let commit = inner.clone();
                    DB::after_commit(move || async move {
                        commit.lock().unwrap().push("inner commit");
                        Ok(())
                    })
                    .await?;
                    DB::after_rollback(move || async move {
                        inner.lock().unwrap().push("inner rollback");
                        Ok(())
                    })
                    .await?;
                    Err::<(), _>(FrameworkError::bad_request("inner failure"))
                })
            })
            .await
            .unwrap_err();
            assert_eq!(failed.status_code(), 400);
            assert_eq!(DB::transaction_level(), 1);
            assert_eq!(*sink.lock().unwrap(), vec!["inner rollback"]);
            assert_eq!(
                DB::table("gap_rows")
                    .filter("id", 2)
                    .max::<i64>("score")
                    .await?,
                Some(20)
            );
            DB::table("gap_rows")
                .filter("id", 3)
                .update(attrs! { score: 33 })
                .await?;
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .unwrap();
    assert_eq!(DB::transaction_level(), 0);
    assert_eq!(
        *calls.lock().unwrap(),
        vec!["inner rollback", "outer commit"]
    );
    assert_eq!(
        DB::table("gap_rows").max::<i64>("score").await.unwrap(),
        Some(33)
    );
}

#[tokio::test]
async fn successful_inner_callbacks_wait_for_outer_commit() {
    let _fx = fixture().await;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let sink = calls.clone();
    DB::transaction(move |_tx| {
        Box::pin(async move {
            let inner = sink.clone();
            DB::transaction(move |_tx| {
                Box::pin(async move {
                    DB::after_commit(move || async move {
                        inner.lock().unwrap().push("commit");
                        Ok(())
                    })
                    .await?;
                    Ok::<(), FrameworkError>(())
                })
            })
            .await?;
            assert!(sink.lock().unwrap().is_empty());
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .unwrap();
    assert_eq!(*calls.lock().unwrap(), vec!["commit"]);
}

#[tokio::test]
async fn outer_rollback_undoes_released_inner_savepoint_and_runs_its_compensation() {
    let _fx = fixture().await;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let sink = calls.clone();
    let result = DB::transaction(move |_tx| {
        Box::pin(async move {
            DB::transaction(move |_tx| {
                Box::pin(async move {
                    DB::table("gap_rows")
                        .filter("id", 1)
                        .update(attrs! { score: 99 })
                        .await?;
                    let rollback = sink.clone();
                    DB::after_rollback(move || async move {
                        rollback.lock().unwrap().push("rollback");
                        Ok(())
                    })
                    .await?;
                    DB::after_commit(move || async move {
                        sink.lock().unwrap().push("commit");
                        Ok(())
                    })
                    .await?;
                    Ok::<(), FrameworkError>(())
                })
            })
            .await?;
            Err::<(), _>(FrameworkError::bad_request("outer failure"))
        })
    })
    .await;
    assert!(result.is_err());
    assert_eq!(DB::transaction_level(), 0);
    assert_eq!(*calls.lock().unwrap(), vec!["rollback"]);
    assert_eq!(
        DB::table("gap_rows")
            .filter("id", 1)
            .max::<i64>("score")
            .await
            .unwrap(),
        Some(10)
    );
    let outside = calls.clone();
    DB::after_rollback(move || async move {
        outside.lock().unwrap().push("outside");
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(*calls.lock().unwrap(), vec!["rollback"]);
}

#[tokio::test]
async fn before_transaction_listeners_run_in_order_before_begin_and_preserve_errors() {
    let _fx = fixture().await;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let sink = calls.clone();
    DB::before_starting_transaction(move || {
        assert_eq!(DB::transaction_level(), 0);
        sink.lock().unwrap().push("before");
        Ok(())
    });
    let sink = calls.clone();
    DB::transaction(move |_tx| {
        Box::pin(async move {
            sink.lock().unwrap().push("body");
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .unwrap();
    assert_eq!(*calls.lock().unwrap(), vec!["before", "body"]);
    DB::before_starting_transaction(|| Err(FrameworkError::domain("blocked begin", 409)));
    let result = DB::transaction(|_tx| {
        Box::pin(async { Err::<(), _>(FrameworkError::internal("body must not start")) })
    })
    .await;
    let error = result.unwrap_err();
    assert_eq!(error.status_code(), 409);
    assert!(error.to_string().contains("blocked begin"));
    assert_eq!(DB::transaction_level(), 0);
    assert_eq!(
        DB::begin_transaction().await.err().unwrap().status_code(),
        409
    );
    assert_eq!(DB::table("gap_rows").count().await.unwrap(), 3);
}

#[tokio::test]
async fn raw_update_grouping_and_having_evaluate_sql_and_keep_bound_parameters() {
    let _fx = fixture().await;
    let mut attrs = UpdateAttrs::new();
    attrs
        .insert("score", DB::raw("score + 5"))
        .insert("grp", "a");
    assert_eq!(
        DB::table("gap_rows")
            .filter("id", 1)
            .update(attrs)
            .await
            .unwrap(),
        1
    );
    let query = DB::table("gap_rows")
        .select_raw_with_bindings("grp, SUM(score) + ? AS total", vec![2.into()])
        .where_raw("score > ?", vec![0.into()])
        .group_by(DB::raw("upper(grp)"))
        .having_op(DB::raw("SUM(score)"), ">", 32)
        .order_by_raw("SUM(score) + ? DESC", vec![1.into()]);
    let (sql, values) = query.to_sql_for(DbBackend::Postgres).unwrap();
    assert!(sql.contains("SUM(score) + $1"));
    assert!(sql.contains("score > $2"));
    assert!(sql.contains("GROUP BY upper(grp) HAVING SUM(score) > $3"));
    assert!(sql.contains("ORDER BY SUM(score) + $4 DESC"));
    assert_eq!(values, vec![2.into(), 0.into(), 32.into(), 1.into()]);
    assert_eq!(query.get().await.unwrap()[0].get_int("total").unwrap(), 37);
    assert!(
        DB::table("gap_rows")
            .update([("score; DROP TABLE gap_rows", DB::raw("0"))])
            .await
            .is_err()
    );
    assert!(
        DB::table("gap_rows")
            .select_raw_with_bindings("? + ?", vec![1.into()])
            .get()
            .await
            .is_err()
    );
}

#[tokio::test]
async fn table_ranges_max_and_oldest_match_model_shapes() {
    let _fx = fixture().await;
    assert_eq!(
        ids(DB::table("gap_rows").where_between("score", 10..=20)).await,
        vec![1, 2]
    );
    assert_eq!(
        ids(DB::table("gap_rows").where_not_between("score", 10..=20)).await,
        vec![3]
    );
    assert_eq!(
        ids(DB::table("gap_rows")
            .filter("id", 3)
            .or_where_between("score", 10..=20))
        .await,
        vec![1, 2, 3]
    );
    assert_eq!(
        ids(DB::table("gap_rows")
            .filter("id", 1)
            .or_where_not_between("score", 10..=20))
        .await,
        vec![1, 3]
    );
    assert_eq!(
        ids(DB::table("gap_rows")
            .filter("id", 1)
            .where_between("score", 10..=20)
            .or_where_between("score", 30..=30)
            .filter("grp", "x"))
        .await,
        vec![1]
    );
    assert_eq!(
        ids(DB::table("gap_rows").where_between(DB::raw("score + 1"), 11..=21)).await,
        vec![1, 2]
    );
    assert_eq!(
        ids(DB::table("gap_rows").where_between(
            DB::table("gap_rows").select(["score"]).filter("id", 1),
            10..=10
        ))
        .await,
        vec![1, 2, 3]
    );
    assert!(
        ids(DB::table("gap_rows").where_between("score", std::ops::RangeInclusive::new(20, 10)))
            .await
            .is_empty()
    );
    assert_eq!(
        DB::table("gap_rows").max::<i64>("score").await.unwrap(),
        Some(30)
    );
    assert_eq!(
        DB::table("gap_rows")
            .max::<i64>(DB::raw("score + 1"))
            .await
            .unwrap(),
        Some(31)
    );
    assert_eq!(
        DB::table("gap_rows")
            .max::<i64>(DB::table("gap_rows").select(["score"]).filter("id", 2))
            .await
            .unwrap(),
        Some(20)
    );
    assert_eq!(
        DB::table("gap_rows")
            .filter("id", 99)
            .max::<i64>("score")
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        DB::table("gap_rows")
            .oldest()
            .first()
            .await
            .unwrap()
            .unwrap()
            .get_int("id")
            .unwrap(),
        2
    );
    assert_eq!(
        DB::table("gap_rows")
            .oldest_by(DB::raw("-score"))
            .first()
            .await
            .unwrap()
            .unwrap()
            .get_int("id")
            .unwrap(),
        3
    );
    for query in [
        DB::table("gap_rows").where_not_between("score; --", 1..=2),
        DB::table("gap_rows").group_by("grp; --"),
        DB::table("gap_rows").oldest_by("score; --"),
    ] {
        assert!(query.get().await.is_err());
    }
    assert!(DB::table("gap_rows").max::<i64>("score; --").await.is_err());
}

#[tokio::test]
async fn table_random_seed_uses_engine_specific_sql_and_sqlite_returns_each_row() {
    let _fx = fixture().await;
    let query = || DB::table("gap_rows").in_random_order(42);
    for backend in [DbBackend::MySql, DbBackend::Postgres] {
        let first = query().to_sql_for(backend).unwrap();
        assert_eq!(first, query().to_sql_for(backend).unwrap());
        assert!(first.1.is_empty());
        if backend == DbBackend::MySql {
            assert!(first.0.contains("ORDER BY RAND(42)"));
        } else {
            assert!(first.0.contains("SELECT setseed("));
            assert!(first.0.contains("ORDER BY CASE WHEN"));
            assert!(first.0.contains("THEN RANDOM() ELSE RANDOM() END"));
            assert!(!first.0.contains("CROSS JOIN"));
        }
    }
    for seed in [None, Some(42), Some(u64::MAX)] {
        let mut rows: Vec<_> = DB::table("gap_rows")
            .in_random_order(seed)
            .get()
            .await
            .unwrap()
            .iter()
            .map(|r| r.get_int("id").unwrap())
            .collect();
        rows.sort_unstable();
        assert_eq!(rows, vec![1, 2, 3]);
    }
    assert!(
        DB::table("gap_rows")
            .filter("id", 99)
            .in_random_order(0)
            .get()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn nested_rollback_callback_can_query_the_still_open_outer_transaction() {
    let _fx = fixture().await;
    DB::transaction(|_tx| {
        Box::pin(async {
            let result = DB::transaction(|_tx| {
                Box::pin(async {
                    DB::after_rollback(|| async {
                        assert_eq!(DB::transaction_level(), 1);
                        DB::table("gap_rows")
                            .filter("id", 1)
                            .update(attrs! { score: 77 })
                            .await?;
                        Ok(())
                    })
                    .await?;
                    Err::<(), _>(FrameworkError::bad_request("rollback child"))
                })
            })
            .await;
            assert!(result.is_err());
            assert_eq!(
                DB::table("gap_rows")
                    .filter("id", 1)
                    .max::<i64>("score")
                    .await?,
                Some(77)
            );
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cancelled_inner_transaction_finishes_rollback_before_outer_outcome() {
    let _fx = fixture().await;
    let rolled_back = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let sink = rolled_back.clone();
    let result = DB::transaction(move |_tx| {
        Box::pin(async move {
            let child = DB::transaction(move |_tx| {
                Box::pin(async move {
                    DB::table("gap_rows")
                        .filter("id", 1)
                        .update(attrs! { score: 99 })
                        .await?;
                    DB::after_rollback(move || async move {
                        tokio::task::yield_now().await;
                        sink.store(true, std::sync::atomic::Ordering::SeqCst);
                        Ok(())
                    })
                    .await?;
                    std::future::pending::<Result<(), FrameworkError>>().await
                })
            });
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(20), child)
                    .await
                    .is_err()
            );
            Err::<(), _>(FrameworkError::bad_request("rollback parent"))
        })
    })
    .await;
    assert!(result.is_err());
    assert!(rolled_back.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(DB::transaction_level(), 0);
    assert_eq!(
        DB::table("gap_rows")
            .filter("id", 1)
            .max::<i64>("score")
            .await
            .unwrap(),
        Some(10)
    );
}

#[tokio::test]
async fn transaction_listeners_observe_the_new_scope_and_nested_lifecycle() {
    use suprnova::{EventFacade, TransactionBeginning, TransactionCommitted};
    let _fx = fixture().await;
    let before = Arc::new(Mutex::new(Vec::new()));
    let sink = before.clone();
    DB::before_starting_transaction(move || {
        sink.lock().unwrap().push(DB::transaction_level());
        Ok(())
    });
    let began = Arc::new(Mutex::new(Vec::new()));
    let committed = Arc::new(Mutex::new(Vec::new()));
    struct BeginCapture(Arc<Mutex<Vec<usize>>>);
    #[suprnova::async_trait]
    impl suprnova::Listener<TransactionBeginning> for BeginCapture {
        async fn handle(&self, _event: &TransactionBeginning) -> Result<(), FrameworkError> {
            let level = DB::transaction_level();
            self.0.lock().unwrap().push(level);
            if level > 0 {
                assert_eq!(DB::table("gap_rows").count().await?, 3);
            }
            Ok(())
        }
    }
    struct CommitCapture(Arc<Mutex<Vec<usize>>>);
    #[suprnova::async_trait]
    impl suprnova::Listener<TransactionCommitted> for CommitCapture {
        async fn handle(&self, _event: &TransactionCommitted) -> Result<(), FrameworkError> {
            self.0.lock().unwrap().push(DB::transaction_level());
            Ok(())
        }
    }
    EventFacade::listen::<TransactionBeginning, _>(Arc::new(BeginCapture(began.clone()))).await;
    EventFacade::listen::<TransactionCommitted, _>(Arc::new(CommitCapture(committed.clone())))
        .await;
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        DB::transaction(|_tx| {
            Box::pin(async {
                DB::transaction(|_tx| Box::pin(async { Ok::<(), FrameworkError>(()) })).await?;
                Ok::<(), FrameworkError>(())
            })
        }),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(*before.lock().unwrap(), vec![0, 1]);
    assert_eq!(*began.lock().unwrap(), vec![1, 2]);
    assert_eq!(*committed.lock().unwrap(), vec![1, 0]);
    EventFacade::forget::<TransactionBeginning>();
    EventFacade::forget::<TransactionCommitted>();
}

#[tokio::test]
async fn nested_db_transaction_uses_a_savepoint() {
    let _fx = fixture().await;
    DB::transaction(|_tx| {
        Box::pin(async move {
            assert_eq!(DB::transaction_level(), 1);
            DB::transaction(|_tx| {
                Box::pin(async move {
                    assert_eq!(DB::transaction_level(), 2);
                    Ok::<(), FrameworkError>(())
                })
            })
            .await?;
            assert_eq!(DB::transaction_level(), 1);
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .unwrap();
    assert_eq!(DB::transaction_level(), 0);
}
