# LADSPA ABI header

`ladspa.h` is the LADSPA 1.1 SDK header by Richard W.E. Furse, Paul Barton-Davis
and Stefan Westerfeld, from <https://www.ladspa.org/ladspa_sdk/ladspa.h.txt>.
Retrieved 2026-10-22; trailing whitespace was removed, with no semantic changes.
The copyright/license notice remains in the header. `COPYING` contains LGPL-2.1;
the header allows LGPL-2.1-or-later. ReSolibre's adjacent `tap.c` is original
GPL-3.0-only code and uses this ABI to export an application-owned filter.

This is a build input, not a third-party plugin bundle. The Linux adapter embeds
its compiled filter and loads it from sealed anonymous memory. Windows/mobile
loaders and package execution remain unvalidated.
