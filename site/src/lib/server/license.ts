// A phrase each of these carries and no other does, which is all it takes to name the common ones.
const KNOWN: [string, RegExp][] = [
  ['Apache-2.0', /Apache License\s+Version 2\.0/i],
  ['MIT', /Permission is hereby granted, free of charge/i],
  ['BSD-3-Clause', /may be used to endorse or promote products/i],
  ['BSD-2-Clause', /Redistributions in binary form must reproduce/i],
  ['ISC', /Permission to use, copy, modify, and\/or distribute/i],
  ['MPL-2.0', /Mozilla Public License,? Version 2\.0/i],
  ['AGPL-3.0', /GNU AFFERO GENERAL PUBLIC LICENSE/i],
  ['GPL-3.0', /GNU GENERAL PUBLIC LICENSE\s+Version 3/i],
  ['LGPL-3.0', /GNU LESSER GENERAL PUBLIC LICENSE/i],
  ['Unlicense', /free and unencumbered software released into the public domain/i]
]

/* The license a notice reads as, null when nothing matches, since a package with an unread license is not a package without one. */
export const identify = (notice: string) => KNOWN.find(([, mark]) => mark.test(notice))?.[0] ?? null
