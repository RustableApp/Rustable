//! Disposable-server checks of indirect changes made by reviewed parent edits.
use super::*;

#[test]
#[ignore = "requires disposable MySQL/MariaDB fixture"]
fn live_review_foreign_key_cascades_commit_and_rollback_with_parent() {
    let port = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
    assert!([43316,43317].contains(&port));
    let config = MysqlConfig { host:"127.0.0.1".into(),port,user:"rustable".into(),
        password:"rustable-test-only".into(),database:"rustable_test".into(),tls:false,..Default::default() };
    let mut driver = MysqlDriver::connect(&config).unwrap();
    let mut observer = Conn::new(OptsBuilder::from(&config)).unwrap();
    for table in ["cascade_review_child","cascade_review_nullable","cascade_review_parent","cascade_review_audit"] {
        observer.query_drop(format!("DROP TABLE IF EXISTS {table}")).unwrap();
    }
    observer.query_drop("CREATE TABLE cascade_review_parent(id INT PRIMARY KEY) ENGINE=InnoDB").unwrap();
    observer.query_drop("CREATE TABLE cascade_review_child(id INT PRIMARY KEY, parent_id INT, parent_twice INT GENERATED ALWAYS AS (parent_id*2) VIRTUAL, row_twice INT GENERATED ALWAYS AS (id*2) STORED, FOREIGN KEY(parent_id) REFERENCES cascade_review_parent(id) ON UPDATE CASCADE ON DELETE CASCADE) ENGINE=InnoDB").unwrap();
    observer.query_drop("CREATE TABLE cascade_review_nullable(id INT PRIMARY KEY, parent_id INT, parent_twice INT GENERATED ALWAYS AS (parent_id*2) VIRTUAL, FOREIGN KEY(parent_id) REFERENCES cascade_review_parent(id) ON UPDATE SET NULL ON DELETE SET NULL) ENGINE=InnoDB").unwrap();
    // A nontransactional audit makes an accidental child-trigger invocation
    // observable even if the managed transaction subsequently rolls back.
    observer.query_drop("CREATE TABLE cascade_review_audit(event VARCHAR(20)) ENGINE=MyISAM").unwrap();
    for (trigger,event) in [("cascade_review_update","UPDATE"),("cascade_review_delete","DELETE")] {
        observer.query_drop(format!("CREATE TRIGGER {trigger} AFTER {event} ON cascade_review_child FOR EACH ROW INSERT INTO cascade_review_audit VALUES('{event}')")).unwrap();
    }
    observer.query_drop("INSERT INTO cascade_review_parent VALUES(1),(2)").unwrap();
    observer.query_drop("INSERT INTO cascade_review_child(id,parent_id) VALUES(10,1),(20,2)").unwrap();
    observer.query_drop("INSERT INTO cascade_review_nullable(id,parent_id) VALUES(10,1),(20,2)").unwrap();
    let original = |driver: &mut MysqlDriver, id| {
        driver.browse(&Browse { table:"cascade_review_parent".into(),..Default::default() }).unwrap()
            .grid.rows.iter().find(|row| row[0]==Value::Integer(id)).unwrap().clone()
    };
    let update = Change::Cell { original_columns: None, table:"cascade_review_parent".into(),column:"id".into(),
        original:original(&mut driver,1),value:Value::Integer(3) };
    let duplicate = Change::InsertRow { id:1,table:"cascade_review_parent".into(),
        fields:vec![("id".into(),Value::Integer(2))] };
    let plan = driver.plan(&[update.clone(),duplicate.clone()]).unwrap();
    assert!(plan.statements[0].sql.starts_with("UPDATE "));
    assert!(driver.apply(&plan).is_err());
    assert_eq!(driver.transaction_state(),TransactionState::Idle);
    for table in ["cascade_review_child","cascade_review_nullable"] {
        assert_eq!(observer.query_first::<i32,_>(format!("SELECT parent_id FROM {table} WHERE id=10")).unwrap(),Some(1));
    }
    assert_eq!(observer.query_first::<(i32,i32),_>("SELECT parent_twice,row_twice FROM cascade_review_child WHERE id=10").unwrap(),Some((2,20)));
    let plan = driver.plan(&[update]).unwrap();
    driver.apply(&plan).unwrap();
    assert_eq!(observer.query_first::<i32,_>("SELECT parent_id FROM cascade_review_child WHERE id=10").unwrap(),Some(3));
    assert_eq!(observer.query_first::<Option<i32>,_>("SELECT parent_id FROM cascade_review_nullable WHERE id=10").unwrap(),Some(None));
    assert_eq!(observer.query_first::<(i32,i32),_>("SELECT parent_twice,row_twice FROM cascade_review_child WHERE id=10").unwrap(),Some((6,20)));
    assert_eq!(observer.query_first::<Option<i32>,_>("SELECT parent_twice FROM cascade_review_nullable WHERE id=10").unwrap(),Some(None));
    let deletion = Change::DeleteRow { table:"cascade_review_parent".into(),original:original(&mut driver,2) };
    // A later duplicate of the remaining parent forces rollback of DELETE,
    // the deleted child, and SET NULL on the other child's foreign key.
    let duplicate = Change::InsertRow { id:2,table:"cascade_review_parent".into(),
        fields:vec![("id".into(),Value::Integer(3))] };
    let plan = driver.plan(&[deletion.clone(),duplicate]).unwrap();
    assert!(plan.statements[0].sql.starts_with("DELETE "));
    assert!(driver.apply(&plan).is_err());
    assert_eq!(driver.transaction_state(),TransactionState::Idle);
    for table in ["cascade_review_child","cascade_review_nullable"] {
        assert_eq!(observer.query_first::<i32,_>(format!("SELECT parent_id FROM {table} WHERE id=20")).unwrap(),Some(2));
    }
    assert_eq!(observer.query_first::<i32,_>("SELECT parent_twice FROM cascade_review_nullable WHERE id=20").unwrap(),Some(4));
    let plan = driver.plan(&[deletion]).unwrap();
    driver.apply(&plan).unwrap();
    assert_eq!(observer.query_first::<u64,_>("SELECT COUNT(*) FROM cascade_review_child WHERE id=20").unwrap(),Some(0));
    assert_eq!(observer.query_first::<Option<i32>,_>("SELECT parent_id FROM cascade_review_nullable WHERE id=20").unwrap(),Some(None));
    assert_eq!(observer.query_first::<Option<i32>,_>("SELECT parent_twice FROM cascade_review_nullable WHERE id=20").unwrap(),Some(None));
    assert_eq!(observer.query_first::<u64,_>("SELECT COUNT(*) FROM cascade_review_audit").unwrap(),Some(0));
    // Demonstrate that the audit triggers themselves work for a direct change.
    observer.query_drop("UPDATE cascade_review_child SET parent_id=3 WHERE id=10").unwrap();
    assert_eq!(observer.query_first::<u64,_>("SELECT COUNT(*) FROM cascade_review_audit").unwrap(),Some(1));
    for table in ["cascade_review_child","cascade_review_nullable","cascade_review_parent","cascade_review_audit"] {
        observer.query_drop(format!("DROP TABLE {table}")).unwrap();
    }
}
