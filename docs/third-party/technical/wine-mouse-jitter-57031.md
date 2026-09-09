# WineHQ — mouse-jitter regression report

- Original title: Bug 57031: Mouse tremble in Robin Hood: The Legend of Sherwood (GoG)
- Source: [Original publication](https://list.winehq.org/hyperkitty/list/wine-bugs%40list.winehq.org/thread/RYUGTWZHJ5GI3KLEIZFP4KPUVJATJI3S/)
- Author / publisher: Flaubert, Béla Gyebrószki, joaopa, and other WineHQ contributors
- Language / date: English; opened 2024-08-03; inspected updates through 2026-03-19
- Access: Substantial indexed bug-mail thread inspected; direct reader retrieval failed.
- Checked: 2026-09-09

Flaubert reports jitter during gameplay, but not menus, with Wine 9.1 staging. Wine 9.0 avoids it on the reporter’s setup but retains occasional delays after unpausing. Béla Gyebrószki reproduces a pronounced regression with the demo and traces it to commit 5b833c83beadcad2ace5f27e95554c164f6f7c86 between Wine 9.3 and 9.4, while noting earlier pointer oddities.

joaopa reports the issue with Wine 11.3 in February 2026. The inspected thread does not show a fix. [Bug 57031](https://bugs.winehq.org/show_bug.cgi?id=57031) is the canonical tracker; referenced pause-delay bug 39513 could not be retrieved. These are contributors’ tests, not local verification.
