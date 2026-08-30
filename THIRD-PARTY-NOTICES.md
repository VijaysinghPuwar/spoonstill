# Third-party notices

spoonstill is distributed as a single binary, and some of what is inside it
belongs to other people. This file travels with every release for that reason —
it is not a formality, it is a condition of one of the licences below (D-124).

If you are reading this inside a release archive: the software it accompanies is
`still` (the command line) and `spoonstill` (the window). Everything here
describes material **embedded in those binaries**, not something you need to
install.

---

## Inter — SIL Open Font License 1.1

Three weights of Inter — Regular, SemiBold and Bold — are compiled into the
binary with `include_bytes!` and used to draw burned-in subtitles (D-106). They
are there because `brew install ffmpeg` ships without `libass` and without
`libfreetype`, so neither the `subtitles` nor the `drawtext` filter exists on
the FFmpeg this program tells operators to install; spoonstill rasterizes the
text itself instead.

The OFL asks, in condition 2, that *"each copy contains the above copyright
notice and this license"*. A copy of spoonstill contains the font, so it
contains this:

```
Copyright 2020 The Inter Project Authors (https://github.com/rsms/inter)

This Font Software is licensed under the SIL Open Font License, Version 1.1.
This license is copied below, and is also available with a FAQ at:
https://scripts.sil.org/OFL


-----------------------------------------------------------
SIL OPEN FONT LICENSE Version 1.1 - 26 February 2007
-----------------------------------------------------------

PREAMBLE
The goals of the Open Font License (OFL) are to stimulate worldwide
development of collaborative font projects, to support the font creation
efforts of academic and linguistic communities, and to provide a free and
open framework in which fonts may be shared and improved in partnership
with others.

The OFL allows the licensed fonts to be used, studied, modified and
redistributed freely as long as they are not sold by themselves. The
fonts, including any derivative works, can be bundled, embedded, 
redistributed and/or sold with any software provided that any reserved
names are not used by derivative works. The fonts and derivatives,
however, cannot be released under any other type of license. The
requirement for fonts to remain under this license does not apply
to any document created using the fonts or their derivatives.

DEFINITIONS
"Font Software" refers to the set of files released by the Copyright
Holder(s) under this license and clearly marked as such. This may
include source files, build scripts and documentation.

"Reserved Font Name" refers to any names specified as such after the
copyright statement(s).

"Original Version" refers to the collection of Font Software components as
distributed by the Copyright Holder(s).

"Modified Version" refers to any derivative made by adding to, deleting,
or substituting -- in part or in whole -- any of the components of the
Original Version, by changing formats or by porting the Font Software to a
new environment.

"Author" refers to any designer, engineer, programmer, technical
writer or other person who contributed to the Font Software.

PERMISSION & CONDITIONS
Permission is hereby granted, free of charge, to any person obtaining
a copy of the Font Software, to use, study, copy, merge, embed, modify,
redistribute, and sell modified and unmodified copies of the Font
Software, subject to the following conditions:

1) Neither the Font Software nor any of its individual components,
in Original or Modified Versions, may be sold by itself.

2) Original or Modified Versions of the Font Software may be bundled,
redistributed and/or sold with any software, provided that each copy
contains the above copyright notice and this license. These can be
included either as stand-alone text files, human-readable headers or
in the appropriate machine-readable metadata fields within text or
binary files as long as those fields can be easily viewed by the user.

3) No Modified Version of the Font Software may use the Reserved Font
Name(s) unless explicit written permission is granted by the corresponding
Copyright Holder. This restriction only applies to the primary font name as
presented to the users.

4) The name(s) of the Copyright Holder(s) or the Author(s) of the Font
Software shall not be used to promote, endorse or advertise any
Modified Version, except to acknowledge the contribution(s) of the
Copyright Holder(s) and the Author(s) or with their explicit written
permission.

5) The Font Software, modified or unmodified, in part or in whole,
must be distributed entirely under this license, and must not be
distributed under any other license. The requirement for fonts to
remain under this license does not apply to any document created
using the Font Software.

TERMINATION
This license becomes null and void if any of the above conditions are
not met.

DISCLAIMER
THE FONT SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO ANY WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT
OF COPYRIGHT, PATENT, TRADEMARK, OR OTHER RIGHT. IN NO EVENT SHALL THE
COPYRIGHT HOLDER BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY,
INCLUDING ANY GENERAL, SPECIAL, INDIRECT, INCIDENTAL, OR CONSEQUENTIAL
DAMAGES, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
FROM, OUT OF THE USE OR INABILITY TO USE THE FONT SOFTWARE OR FROM
OTHER DEALINGS IN THE FONT SOFTWARE.
```

---

## Rust dependencies

The crates spoonstill links are listed with their versions in `Cargo.lock`, and
their licences in each crate's own repository — overwhelmingly MIT or
Apache-2.0, both of which ask that their notice accompany a binary
distribution.

**This file does not yet reproduce them**, and that is a known gap rather than a
judgement that they do not apply. Doing it properly means generating the roll-up
from `Cargo.lock` at release time — `cargo-about` is the usual tool — so that it
cannot drift from what was actually linked. Written down here so the next person
finds the gap rather than assuming this file is complete.

---

## FFmpeg

**Not distributed with spoonstill.** FFmpeg is the operator's own installation,
found on their machine at run time (D-012, D-103), and the installers hand the
job to Homebrew or winget rather than fetching a build nobody chose. If a
release ever bundles FFmpeg, D-062 is the decision that governs which build it
may be, and this file gains a section.
