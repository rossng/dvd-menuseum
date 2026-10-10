// The licences of what the site serves, for the notices page (linked from
// every page's footer). The crates compiled into the player come from
// src/data/notices.json, written by scripts/notices.py.

import notices from "../data/notices.json";

export const BLUE_OAK = { name: "Blue Oak Model License 1.0.0", url: "https://blueoakcouncil.org/license/1.0.0" };
export const GPL = { name: "GPL-2.0-or-later", url: "https://www.gnu.org/licenses/old-licenses/gpl-2.0.html" };
export const OFL = { name: "SIL Open Font License 1.1", url: "https://openfontlicense.org" };

// Code the player's navigation follows (crates/engine), by its authors.
export const LIBDVDNAV = ["Copyright (C) 2000, 2001 Håkan Hjort", "Copyright (C) 2000, 2001 Rich Wareham"];

// Groups of crates sharing a licence and its text.
export const crateLicences = notices.licences;

// What the site bundles from npm.
export const bundled = [
  { name: "Seal of Slop", url: "https://www.rossng.eu/seal-of-slop/", licence: BLUE_OAK },
  { name: "Open Sans typeface", url: "https://github.com/googlefonts/opensans", copyright: "Copyright 2020 The Open Sans Project Authors", licence: OFL },
];
