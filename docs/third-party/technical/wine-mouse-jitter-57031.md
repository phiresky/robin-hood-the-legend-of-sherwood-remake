# WineHQ Bug 57031 — Mouse “tremble” in Robin Hood: The Legend of Sherwood (GoG)

- **Original publication:** [WineHQ HyperKitty thread](https://list.winehq.org/hyperkitty/list/wine-bugs%40list.winehq.org/thread/RYUGTWZHJ5GI3KLEIZFP4KPUVJATJI3S/)
- **Canonical tracker:** [Bug 57031](https://bugs.winehq.org/show_bug.cgi?id=57031)
- **Mailing list:** `wine-bugs@list.winehq.org`
- **Thread opened:** August 3, 2024
- **Thread last active:** March 19, 2026 (in the retrieved page’s thread data)
- **Language:** English

## Captured messages

The supplied browser capture contains the first message and eight replies/events. The message bodies below retain the captured text, links, authors, dates, and times, with only the Bugzilla mail footer and HyperKitty page chrome removed.

### Message 1 — WineHQ Bugzilla — August 3, 2024, 6:21 p.m.

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

### Message 2 — WineHQ Bugzilla — August 4, 2024, 10:31 a.m.

<https://bugs.winehq.org/show_bug.cgi?id=57031>

Béla Gyebrószki <gyebro69(a)gmail.com> changed:

```text
           What    |Removed                     |Added
----------------------------------------------------------------------------
                URL|                            |https://archive.org/downloa
                   |                            |d/RobinHoodTheLegendOfSherw
                   |                            |oodDemo/Setup_us.exe
                 CC|                            |gyebro69(a)gmail.com
           Keywords|                            |download
```

--- Comment #1 from Béla Gyebrószki <gyebro69(a)gmail.com> ---

I'm pretty sure the problem is there in vanilla Wine too as anyone can test and
reproduce the problem with the demo version:
<https://archive.org/download/RobinHoodTheLegendOfSherwoodDemo/Setup_us.exe>

`Setup_us.exe  (81 M)`

`md5sum: a8c4df5cbf009f3381ba582e6fe6c5f2`

As for being a regression, I ended up my regression test with this (between
wine-9.3 and 9.4):

`commit 5b833c83beadcad2ace5f27e95554c164f6f7c86`

`server: Stop waiting on LL-hooks for non-injected input.`

I must say, even before that commit there was something odd about the way the
mouse pointer moved when I alt-tabbed and back to the game window in virtual
desktop mode, but commit 5b833c83 is the one which makes the problem highly
noticeable.

`wine-9.14-99-geb7bbf9858b`

`X.Org X Server 1.21.1.13`

### Message 3 — WineHQ Bugzilla — August 4, 2024, 10:52 a.m.

<https://bugs.winehq.org/show_bug.cgi?id=57031>

--- Comment #2 from Flaubert <flaubertSt(a)gmail.com> ---

Versión that fails forms is also 9.14 (staging). Versión 9.0 (system, in Ubuntu
22.04) works with just ocasional  "lag after pauses" bug.

### Message 4 — WineHQ Bugzilla — February 25, 2026, 9:37 p.m.

<http://bugs.winehq.org/show_bug.cgi?id=57031>

Lukáš Linhart <l.linhos@gmail.com> changed:

```text
                 What    |Removed                     |Added
----------------------------------------------------------------------------
                 CC|                            |l.linhos@gmail.com
```

### Message 5 — WineHQ Bugzilla — February 25, 2026, 10:26 p.m.

<http://bugs.winehq.org/show_bug.cgi?id=57031>

joaopa <jeremielapuree@yahoo.fr> changed:

```text
           What    |Removed                     |Added
----------------------------------------------------------------------------
                 CC|                            |jeremielapuree@yahoo.fr
```

--- Comment #3 from joaopa <jeremielapuree@yahoo.fr> ---

I confirm the bug with wine-11.3

### Message 6 — WineHQ Bugzilla — February 27, 2026, 7:26 p.m.

<http://bugs.winehq.org/show_bug.cgi?id=57031>

--- Comment #4 from Lukáš Linhart <l.linhos@gmail.com> ---

I confirm that after reverting the changes from commit
5b833c83beadcad2ace5f27e95554c164f6f7c86 and building the current master
branch, the problem disappeared.

Now it's up to someone who understands queue processing to come up with a fix.

### Message 7 — WineHQ Bugzilla — March 15, 2026, 10:58 a.m.

<http://bugs.winehq.org/show_bug.cgi?id=57031>

y5kXCS6RgtQp <jacobbrett+winehqbugs@jacobbrett.id.au> changed:

```text
           What    |Removed                     |Added
----------------------------------------------------------------------------
                 CC|                            |jacobbrett+winehqbugs@jacob
                   |                            |brett.id.au
```

--- Comment #5 from y5kXCS6RgtQp <jacobbrett+winehqbugs@jacobbrett.id.au> ---

I suspect this issue is related to a similar issue with Star Trek: Away Team.
Working normally with Wine 8.6, but broken in a similar fashion under Wine
10/11.

### Message 8 — WineHQ Bugzilla — March 15, 2026, 11 a.m.

<http://bugs.winehq.org/show_bug.cgi?id=57031>

--- Comment #6 from y5kXCS6RgtQp <jacobbrett+winehqbugs@jacobbrett.id.au> ---

Excuse my last comment, I meant to say "similar issue with Starship Troopers:
Terran Ascendancy". -- I mixed up my test results.

### Message 9 — WineHQ Bugzilla — March 19, 2026, 10:56 p.m.

<http://bugs.winehq.org/show_bug.cgi?id=57031>

Antoine Le Gonidec <accounts.winehq@vv221.fr> changed:

```text
           What    |Removed                     |Added
----------------------------------------------------------------------------
                 CC|                            |accounts.winehq@vv221.fr
```

## Thread metadata

The HyperKitty page reports **8 comments** and **2 participants**, both displayed as “WineHQ Bugzilla.” It also reports an age of 767 days and last activity 174 days ago in the captured page data.
