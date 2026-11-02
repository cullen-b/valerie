# Runs once on the test machine's first login: lets scripts/windows-vm.sh in over SSH and keeps the screen on.
$ErrorActionPreference = "Continue"
$disc = Split-Path $MyInvocation.MyCommand.Path

powercfg /change standby-timeout-ac 0
powercfg /change monitor-timeout-ac 0

Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0
Set-Service sshd -StartupType Automatic
# Keys for administrators live in one shared file, readable only by admins and SYSTEM.
$keys = "$env:ProgramData\ssh\administrators_authorized_keys"
New-Item -ItemType Directory -Force (Split-Path $keys) | Out-Null
Copy-Item (Join-Path $disc "key.pub") $keys
icacls $keys /inheritance:r /grant "Administrators:F" /grant "SYSTEM:F" | Out-Null
New-NetFirewallRule -DisplayName "SSH for the Valerie test machine" -Direction Inbound -Protocol TCP -LocalPort 22 -Action Allow | Out-Null
Start-Service sshd
