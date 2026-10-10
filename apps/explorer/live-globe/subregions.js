// Official UN M49 Sub-region column (not the finer Intermediate Region column).
// Source: https://unstats.un.org/unsd/methodology/m49/overview/
// Anchors are bundled representative artwork, never a node's location.
const groups = [
  ['015', 'Northern Africa', [16, 26]],
  ['202', 'Sub-Saharan Africa', [23, -8]],
  ['021', 'Northern America', [-105, 47]],
  ['419', 'Latin America and the Caribbean', [-73, -10]],
  ['143', 'Central Asia', [67, 43]],
  ['030', 'Eastern Asia', [112, 35]],
  ['035', 'South-eastern Asia', [110, 9]],
  ['034', 'Southern Asia', [77, 22]],
  ['145', 'Western Asia', [44, 30]],
  ['151', 'Eastern Europe', [40, 52]],
  ['154', 'Northern Europe', [11, 61]],
  ['039', 'Southern Europe', [15, 41]],
  ['155', 'Western Europe', [5, 49]],
  ['053', 'Australia and New Zealand', [145, -30]],
  ['054', 'Melanesia', [161, -10]],
  ['057', 'Micronesia', [150, 10]],
  ['061', 'Polynesia', [-155, -15]],
];

export const SUBREGION_CODES = Object.freeze(groups.map(([code]) => code));
export const SUBREGION_NAMES = Object.freeze(Object.fromEntries(groups.map(([code, name]) => [code, name])));
export const SUBREGION_CENTROIDS = Object.freeze(Object.fromEntries(
  groups.map(([code, , anchor]) => [code, Object.freeze(anchor)]),
));
