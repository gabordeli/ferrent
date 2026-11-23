Ferrent for Windows (x86_64)
============================

This ZIP is the manual route. The release also ships
Ferrent-<version>-windows-x86_64-setup.exe, which installs the same files and
registers the service from one wizard; use it if you do not want to run
PowerShell by hand.

Files
-----
  ferrentd.exe        the daemon (BitTorrent engine, gRPC over TLS)
  ferrent-cli.exe     command line client
  ferrent-gui.exe     desktop client (iced)
  libstdc++-6.dll     the C++ runtime ferrentd.exe needs
  nssm.exe            service wrapper (public domain)
  service.ps1         installs/removes the service
  config.example.toml reference configuration

Quick start (daemon as a service)
---------------------------------
Open an elevated PowerShell in this folder:

    powershell -ExecutionPolicy Bypass -File .\service.ps1 install

It creates %ProgramData%\ferrent (data, token, logs, config.toml), registers
the "Ferrent" service, and starts it. The token it prints is what the GUI
needs.

    .\service.ps1 status | stop | start | restart | uninstall

The logs are in %ProgramData%\ferrent\logs. The token is in
%ProgramData%\ferrent\token.

Why a wrapper
-------------
ferrentd saves fastresume on Ctrl-C. NSSM forwards a console stop event to the
child, so a service stop writes fastresume exactly like Ctrl-C in a console
does; a plain `sc stop` would terminate the process instead.

Connecting the GUI
------------------
Start ferrent-gui.exe, open "Servers", add https://127.0.0.1:50051 with the
token above, and accept the certificate fingerprint the daemon printed at
startup (also in %ProgramData%\ferrent\data\tls-fingerprint.txt). The daemon
issues its certificate for localhost, so a remote client needs the CA file from
%ProgramData%\ferrent\data\tls-cert.pem.

Manual run (no service)
-----------------------
    set FERRENT_CONFIG=%ProgramData%\ferrent\config.toml
    ferrentd.exe
