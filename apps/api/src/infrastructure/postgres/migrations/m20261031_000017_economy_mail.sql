-- Phase 5 (design §11.4 row 3, §10.3): mail messages. expires_at is computed by the checkpoint
-- writer in the insert (now() + 360 h, or + 12 h when cod_adena > 0); the zone supplies no time.
CREATE TABLE mail_messages (
    id uuid PRIMARY KEY,
    sender_id uuid NOT NULL REFERENCES characters(id),
    receiver_id uuid NOT NULL REFERENCES characters(id),
    subject text NOT NULL CONSTRAINT mail_messages_subject CHECK (char_length(subject) <= 128),
    body text NOT NULL CONSTRAINT mail_messages_body CHECK (char_length(body) <= 512),
    cod_adena bigint NOT NULL DEFAULT 0 CONSTRAINT mail_messages_cod CHECK (cod_adena BETWEEN 0 AND 99900000000),
    has_attachments boolean NOT NULL DEFAULT false,
    returned boolean NOT NULL DEFAULT false,
    system_kind smallint NOT NULL DEFAULT 0 CONSTRAINT mail_messages_system_kind CHECK (system_kind BETWEEN 0 AND 3),
    read boolean NOT NULL DEFAULT false,
    deleted_by_sender boolean NOT NULL DEFAULT false,
    deleted_by_receiver boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    revision bigint NOT NULL DEFAULT 0 CONSTRAINT mail_messages_revision CHECK (revision >= 0)
);
CREATE INDEX mail_messages_receiver_idx ON mail_messages (receiver_id) WHERE NOT deleted_by_receiver;
CREATE INDEX mail_messages_sender_idx ON mail_messages (sender_id) WHERE NOT deleted_by_sender;
CREATE INDEX mail_messages_expires_idx ON mail_messages (expires_at);
-- Attachment rows reference their message; the append-only history outlives nothing here (RESTRICT).
ALTER TABLE item_instances ADD CONSTRAINT item_instances_mail_fk FOREIGN KEY (mail_id) REFERENCES mail_messages(id) ON DELETE RESTRICT;
