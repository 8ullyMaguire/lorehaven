-- Migration 0071 — M47: delivery_channel on notifications (spec §47.4)
--
-- Adds delivery_channel to notifications table; used by resolve_notification_channel
-- to drop notifications for events with no enabled route.

ALTER TABLE notifications ADD COLUMN delivery_channel TEXT NOT NULL DEFAULT 'in_app';
