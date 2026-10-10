# Security

What local-takkie's encryption protects, what it doesn't, and why. The exact
packet format is in [protocol.md](protocol.md).

## In short

- A channel is **open** by default: anyone on the same network can listen
  and talk on it. Treat an open channel like speaking out loud in the room.
- A channel becomes **private** when everyone on it sets the same
  passphrase (press `P` in the terminal app). Then only people with that
  passphrase can hear the audio or put audio into it.
- Even on a private channel, people on the network can still see **that**
  you are using local-takkie, your display name, your channel number and
  when you talk. Only what you say is hidden.

## How a private channel works

- The passphrase and the channel number are turned into a 32-byte key with
  Argon2id (19 MiB of memory, 2 passes). This is slow on purpose, to make
  guessing passphrases expensive.
- Every packet on the channel (audio, the Hello keep-alive and the Bye when
  leaving) is encrypted and authenticated with ChaCha20-Poly1305. The packet
  header is authenticated too, so nobody can change the channel, the sender
  or the sequence number of a packet without it being rejected.
- Each packet uses a unique nonce made of the sender's random id and its
  sequence number, and receivers refuse a sequence number they have already
  accepted from that sender.

There is no server and no key exchange: the passphrase is the only secret,
and you share it with the others yourself, outside the app.

## What is protected on a private channel

| Threat | Protected? |
|---|---|
| Someone on the Wi-Fi records the traffic to listen to the audio | Yes. Without the passphrase the audio is unreadable. |
| Someone without the passphrase sends audio into the channel | Yes. Their packets fail the check and are dropped. |
| Someone changes a packet on the way | Yes. Any change to header, payload or tag is rejected. |
| Someone records a packet and sends it again later | Yes. Each sequence number is accepted once per sender. |
| Someone without the passphrase sends a fake "Bye" to make a member vanish from your list | Yes. A Bye is only believed if it is sealed with the channel key, or comes from a peer that never seals its packets. |
| Someone joins the channel number with a wrong passphrase | They hear nothing and can't be heard. Both sides are told there is a passphrase mismatch. |

## What is not protected

- **Who is there and when they talk.** Display names, channel numbers and
  sender ids are announced over mDNS and sit in packet headers in the clear.
  Packet timing shows who is transmitting and for how long.
- **Open channels.** No passphrase means no encryption and no check of who
  sent a packet. Anyone on the network can listen, talk, or send a fake Bye
  for another open peer.
- **Anyone who has the passphrase.** All members share one key. A member can
  listen to everything, and can send packets that claim to come from another
  member: the app does not prove *which* member spoke.
- **Weak passphrases.** Someone who records traffic can try passphrases
  offline, as fast as their hardware allows. Argon2id makes each guess cost
  19 MiB and noticeable time, but a short or common passphrase will still
  fall. The salt is fixed per channel number, so guesses can be prepared in
  advance for common passphrases. Use several random words; the app warns
  about passphrases under 12 characters.
- **Old recordings if the passphrase leaks later.** There is no forward
  secrecy: traffic recorded today can be decrypted by anyone who learns the
  passphrase afterwards. Change the passphrase if it may have leaked.
- **Availability.** Anyone on the network can flood it, jam the Wi-Fi, or
  send junk packets. Junk is dropped cheaply, but local-takkie can't stop
  someone from disrupting the network itself.
- **A compromised computer or phone.** Malware or another person at your
  device can read the passphrase from the running app.
- **Peers on other channels.** They are shown in the peer list from the
  cleartext part of their packets (id, channel, address), with the name from
  mDNS. This is shown but not trusted: nothing proves such a peer is who it
  says it is. Fake entries can appear there; they can't be heard and can't
  remove real peers from a private channel.

## Limits to know about

- A sender must not reuse a nonce under one key. Sender ids are 64 random
  bits from the operating system, chosen at every start, so two members
  picking the same one is not a practical concern. The sequence number wraps
  after 2^32 packets, about 2.7 years of non-stop talking in one run; restart
  the app before then.
- The replay window covers the last 64 packets per sender. Audio that
  arrives more than 64 packets late is dropped, which never matters for live
  speech.

## How the app handles the passphrase

- It is kept in memory only, for the current session, and wiped when no
  longer needed. It is never written to the settings file or the log.
- It can't be given as a command-line argument, because that would end up in
  shell history and be visible in the process list. The `TAKKIE_PASSPHRASE`
  environment variable works for the starting channel; other programs
  running as the same user can read environment variables, so prefer typing
  it with `P` on a shared machine.
- While you type it, the screen shows dots.
- Storing it in the operating system's keychain is not implemented yet
  ([#255](https://github.com/martcpp/local-takkie/issues/255)).

## Building blocks

- Argon2id and ChaCha20-Poly1305 from the RustCrypto project (`argon2` and
  `chacha20poly1305` crates). No cryptography is implemented in local-takkie
  itself.
- Test vectors for the key derivation and for a sealed packet are in
  [protocol.md](protocol.md#test-vectors). They were checked against the
  reference Argon2 library and OpenSSL, and the tests fail if the document
  and the code disagree.
- The packet parser and the code that opens sealed packets are fuzzed.

## Reporting a problem

If you find a security problem, please report it privately through
[GitHub's "Report a vulnerability"](https://github.com/martcpp/local-takkie/security/advisories/new)
instead of a public issue.
