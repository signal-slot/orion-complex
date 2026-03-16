CREATE TABLE IF NOT EXISTS usb_attachments (
  id TEXT PRIMARY KEY,
  env_id TEXT NOT NULL,
  vendor_id TEXT NOT NULL,
  product_id TEXT NOT NULL,
  description TEXT,
  attached_at INTEGER NOT NULL,
  FOREIGN KEY (env_id) REFERENCES environments(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_usb_attachments_env ON usb_attachments(env_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_usb_attachments_device ON usb_attachments(vendor_id, product_id);
