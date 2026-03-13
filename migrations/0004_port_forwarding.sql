-- Add port forwarding fields to environments
ALTER TABLE environments ADD COLUMN port_forwarding INTEGER NOT NULL DEFAULT 0;
ALTER TABLE environments ADD COLUMN ssh_host TEXT;
ALTER TABLE environments ADD COLUMN ssh_port INTEGER;
ALTER TABLE environments ADD COLUMN vnc_host TEXT;
ALTER TABLE environments ADD COLUMN vnc_port INTEGER;
