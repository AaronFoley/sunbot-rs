use sea_orm_migration::sea_orm::{ConnectionTrait, Database};
use sunbot_migrations::{Migrator, MigratorTrait};

#[tokio::test]
async fn punishment_schema_enforces_guild_relationships_and_rolls_back() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    db.execute_unprepared("PRAGMA foreign_keys = ON")
        .await
        .unwrap();
    Migrator::up(&db, None).await.unwrap();

    db.execute_unprepared("INSERT INTO guild (id) VALUES (1)")
        .await
        .unwrap();
    for statement in [
        "INSERT INTO punishment_channel (guild, channel) VALUES (1, 123)",
        "INSERT INTO punishment_song (guild, name, url) VALUES (1, 'song', 'https://example.com/song')",
        "INSERT INTO punishment (guild, user, expires_at) VALUES (1, 456, '2030-01-01 00:00:00')",
    ] {
        assert!(db.execute_unprepared(&statement.replace("VALUES (1,", "VALUES (999,")).await.is_err());
        db.execute_unprepared(statement).await.unwrap();
    }
    db.execute_unprepared("DELETE FROM guild WHERE id = 1")
        .await
        .unwrap();
    for table in ["punishment_channel", "punishment_song", "punishment"] {
        let rows = db
            .query_all_raw(sea_orm_migration::sea_orm::Statement::from_string(
                db.get_database_backend(),
                format!("SELECT * FROM {table}"),
            ))
            .await
            .unwrap();
        assert!(rows.is_empty(), "guild deletion must cascade to {table}");
    }
    Migrator::down(&db, None).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
}
