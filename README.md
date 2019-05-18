# whatsappweb-rs (eta's fork)

[![Documentation for this fork](https://img.shields.io/badge/fork%20docs-on%20theta.eu.org-blueviolet.svg)](https://theta.eu.org/stash/ww-rs-docs/whatsappweb/index.html)

## What is this?

This is a fork of [whatsappweb-rs](https://github.com/wiomoc/whatsappweb-rs) by [Christoph Walcher (@wiomoc)](https://github.com/wiomoc).
It's used by [sms-irc](https://git.theta.eu.org/sms-irc.git/about/), and is actively maintained in conjuction with that project.
I didn't write most of this code - and most of the reverse-engineering work for the protocol was done by [sigalor](https://github.com/sigalor),
under the aegis of the [whatsapp-web-reveng](https://github.com/sigalor/whatsapp-web-reveng) project.

## What have you changed?

- Various stability fixes (doesn't error out as easily when WhatAPp send us weird stuff)
- Dependency updates (to work with modern Rust crates)
- New error handling (using `failure` instead of `error-chain`)
- Small bug fixes here and there
- More data exposed (e.g. videos, unimplemented messages, captions)

## Features

*(taken from the original README verbatim)*

* send/receive text-, image-, audiomessages
* create/modify groups
* get userinfo (status, presence, profilepic)
* get contacts, chats
* send/receive message acknowledge
* receive changes from app e.g. (un-)pin, (un-)mute, delete chat
* relogin without scanning qrcode
* query older messages
* reconnect on connection loss


## TODO

- Error messages need to be less stringly-typed
- Message deletions / revocations
- Broadcast lists
- Documentation!
- Code cleanup

## Legal

This code is in no way affiliated with, authorized, maintained, sponsored or endorsed by WhatsApp or any of its
affiliates or subsidiaries. This is an independent and unofficial software. Use at your own risk.
