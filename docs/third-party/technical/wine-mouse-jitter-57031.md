# WineHQ Bug 57031 — Mouse “tremble” in Robin Hood: The Legend of Sherwood (GoG)

- **Original publication:** [WineHQ HyperKitty thread](https://list.winehq.org/hyperkitty/list/wine-bugs%40list.winehq.org/thread/RYUGTWZHJ5GI3KLEIZFP4KPUVJATJI3S/)
- **Canonical tracker:** [Bug 57031](https://bugs.winehq.org/show_bug.cgi?id=57031)
- **Mailing list:** `wine-bugs@list.winehq.org`
- **Thread opened:** August 3, 2024
- **Thread last active:** March 19, 2026 (in the retrieved page’s thread data)
- **Language:** English

## Editorial notes

The supplied HyperKitty HTML contains the first message and a JavaScript lazy-load placeholder for the replies. The supplied TXT rendering contains the same first message and thread metadata, but no reply bodies. The Bugzilla page was unavailable behind an Anubis proof-of-work wall in the supplied retrieval.

## Original text

#### First message — WineHQ Bugzilla, August 3, 2024, 6:21 p.m.

<https://bugs.winehq.org/show_bug.cgi?id=57031>

| Field | Value |
|---|---|
| Bug ID | 57031 |
| Summary | Mouse "tremble" in Robin Hood: The Legend of Sherwood (GoG). |
| Product | Wine-staging |
| Version | 9.1 |
| Hardware | x86-64 |
| OS | Linux |
| Status | UNCONFIRMED |
| Severity | minor |
| Priority | P2 |
| Component | -unknown |
| Assignee | wine-bugs(a)winehq.org |
| Reporter | flaubertSt(a)gmail.com |
| CC | leslie_alistair(a)hotmail.com, z.figura12(a)gmail.com |
| Distribution | --- |

> Installed GoG version of Robin Hood: The Legend of Sherwood. All works, but
> mouse tremble when in game (it DOESN'T with menu screens). Tried "mouse warp
> override -all options-but it didnt solve. Strangely, I changed from
> WINE-9.1-staging to 9.0 stable version and it doesn't tremble anymore, but now
> it has some lag when exit from pause: see Bug 39513).

#### End of captured first message

### Editorial research notes — unavailable reply bodies

The following are **unverified editorial research notes**, retained to record the available thread metadata and findings. They are not captured source text: the mapped HTML contains only a lazy-load placeholder for the replies, and the mapped TXT contains no reply bodies.

1. **August 4, 2024, 10:52 a.m. — Flaubert, Comment #2.** The failing version is also Wine 9.14 staging. Wine 9.0 (the system package on Ubuntu 22.04) works, with only the occasional “lag after pauses” bug.

2. **February 25, 2026, 9:37 p.m. — Lukáš Linhart (`l.linhos@gmail.com`).** Adds himself to CC. No comment text is available in the supplied source files.

3. **February 25, 2026, 10:26 p.m. — joaopa (`jeremielapuree@yahoo.fr`), Comment #3.** Adds himself to CC and confirms the bug with Wine 11.3.

4. **February 27, 2026, 7:26 p.m. — Lukáš Linhart, Comment #4.** Reverting the changes from commit `5b833c83beadcad2ace5f27e95554c164f6f7c86` on the current master branch and rebuilding makes the problem disappear. He says a fix now needs someone who understands queue processing.

5. **March 15, 2026, 10:58 a.m. — y5kXCS6RgtQp (`jacobbrett+winehqbugs@jacobbrett.id.au`), Comment #5.** Adds himself to CC. He suspects a relation to a similar issue with *Star Trek: Away Team*, which works with Wine 8.6 but breaks similarly under Wine 10/11.

6. **March 15, 2026, 11:00 a.m. — y5kXCS6RgtQp, Comment #6.** Corrects the previous comment: the similar issue is with *Starship Troopers: Terran Ascendancy*, not *Star Trek: Away Team*; the test results had been mixed up.

7. **March 19, 2026, 10:56 p.m. — Antoine Le Gonidec (`accounts.winehq@vv221.fr`).** Adds himself to CC. No comment text is available in the supplied source files.

### Thread metadata

The HyperKitty page reports **8 comments** and **2 participants**, both displayed as “WineHQ Bugzilla.” It also reports an age of 767 days and last activity 174 days ago in the captured page data.

### Gaps and related references

- Bugzilla **Comment #1** is not part of the supplied HyperKitty thread: the thread jumps from the creation mail to Comment #2. The conversion notes attribute to Béla Gyebrószki a reproduction with the demo and a bisection to commit `5b833c83beadcad2ace5f27e95554c164f6f7c86` between Wine 9.3 and 9.4, while noting earlier pointer oddities. That report is not present in the supplied thread files and the Bugzilla page could not be read.
- No supplied message reports a fix or a status change away from **UNCONFIRMED**.
- Wine versions named in the available material are 9.0 (works, with pause lag), 9.1 staging and 9.14 staging (fail), and 11.3 (fails). The related-game comparison names Wine 8.6 versus 10/11.
- Bug 39513 is cited by the first message for the pause-exit lag, but its page was not supplied.
