ALTER TABLE environments ADD COLUMN win_install_options TEXT;
UPDATE environments SET win_install_options = '{"bypass_tpm":true,"bypass_secure_boot":true,"bypass_ram":true,"bypass_cpu":true}' WHERE bypass_hw_check = 1;
