use crate::m20220101_000001_guild_table::Guild;
use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(PunishmentChannel::Table)
                    .if_not_exists()
                    .col(pk_auto(PunishmentChannel::Id))
                    .col(big_unsigned_uniq(PunishmentChannel::Guild))
                    .col(big_unsigned_uniq(PunishmentChannel::Channel))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_guild")
                            .from(PunishmentChannel::Table, PunishmentChannel::Guild)
                            .to(Guild::Table, Guild::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_punishment_channel_guild")
                    .if_not_exists()
                    .table(PunishmentChannel::Table)
                    .col(PunishmentChannel::Guild)
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(PunishmentSong::Table)
                    .if_not_exists()
                    .col(pk_auto(PunishmentSong::Id))
                    .col(big_unsigned(PunishmentSong::Guild))
                    .col(string(PunishmentSong::Name))
                    .col(string(PunishmentSong::Url))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_guild")
                            .from(PunishmentSong::Table, PunishmentSong::Guild)
                            .to(Guild::Table, Guild::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_punishment_song_guild")
                    .if_not_exists()
                    .table(PunishmentSong::Table)
                    .col(PunishmentSong::Guild)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_unique_punishment_song_guild_url")
                    .if_not_exists()
                    .unique()
                    .table(PunishmentSong::Table)
                    .col(PunishmentSong::Guild)
                    .col(PunishmentSong::Url)
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Punishment::Table)
                    .if_not_exists()
                    .col(pk_auto(Punishment::Id))
                    .col(big_unsigned(Punishment::Guild))
                    .col(big_unsigned(Punishment::User))
                    .col(date_time(Punishment::ExpiresAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_guild")
                            .from(Punishment::Table, Punishment::Guild)
                            .to(Guild::Table, Guild::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_punishment_guild")
                    .if_not_exists()
                    .table(Punishment::Table)
                    .col(Punishment::Guild)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_punishment_user")
                    .if_not_exists()
                    .table(Punishment::Table)
                    .col(Punishment::User)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_punishment_channel_guild")
                    .table(PunishmentChannel::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("idx_punishment_song_guild")
                    .table(PunishmentSong::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("idx_unique_punishment_song_guild_url")
                    .table(PunishmentSong::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("idx_punishment_guild")
                    .table(Punishment::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("idx_punishment_user")
                    .table(Punishment::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;

        manager
            .drop_table(
                Table::drop()
                    .table(PunishmentChannel::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(PunishmentSong::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(Punishment::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
pub enum PunishmentChannel {
    Table,
    Id,
    Guild,
    Channel,
}

#[derive(DeriveIden)]
pub enum PunishmentSong {
    Table,
    Id,
    Guild,
    Name,
    Url,
}

#[derive(DeriveIden)]
pub enum Punishment {
    Table,
    Id,
    Guild,
    User,
    ExpiresAt,
}
