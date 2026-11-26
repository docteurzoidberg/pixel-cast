# The E1.31 standard

pixel-cast implements **ANSI E1.31-2025, "Entertainment Technology — Lightweight streaming protocol for transport
of DMX512 using ACN"** (sACN), published by ESTA (document CP/2014-1009r8). Section and table numbers quoted in the
code and the plan (such as Table 4-1, the data packet, or §6.2.6, its options) refer to this edition.

## Where to get it

The standard is © ESTA. It is a **free download** from ESTA's Technical Standards Program, after signing in with an
e-mail address:

- <https://tsp.esta.org/tsp/documents/published_docs.php> (published standards; see also
  <https://tsp.esta.org/freestandards>)

It may be downloaded for free but not redistributed, so it is **not included in this repository**. It is also sold
by the ANSI webstore: <https://webstore.ansi.org/standards/esta/ansie1312025>.

Earlier editions (2009, 2016, 2018) use the same data packet layout; 2016 added universe synchronization and
discovery, which pixel-cast does not send.

## Local copy

To work on pixel-cast, put the PDF in this folder as `docs/standard/ANSI_E1.31-2025.pdf`. PDF files here are ignored
by git: the copy stays on your machine.

## Sections pixel-cast relies on

| Where | What |
|---|---|
| Table 4-1 | data packet layout, octet by octet |
| §5.6 | CID: the same for a source across restarts |
| §6.2.2 | source name: UTF-8, null-terminated, 64 octets |
| §6.2.3 | priority: 0 to 200, 100 by default |
| §6.2.5, §6.7.2 | sequence numbers, one counter per universe |
| §6.2.6 | options: Preview_Data, Stream_Terminated (three packets when a source stops a universe; their values are ignored) |
| §6.2.7 | universes 1 to 63999 |
| §6.6.2 | unchanged data: three identical packets, then one keep-alive packet every 800 to 1000 ms |
| §6.7.1, annex A | network data loss: receivers drop a source after 2.5 s (E131_NETWORK_DATA_LOSS_TIMEOUT) |
| §9.1.1, Table 9-10 | multicast addresses 239.255.<high>.<low> |
