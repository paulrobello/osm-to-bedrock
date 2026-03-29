export interface Preset {
  name: string;
  scale: number;
  buildingHeight: number;
  seaLevel: number;
  signs: boolean;
}

export const PRESETS: Preset[] = [
  { name: 'Detailed City', scale: 1.0, buildingHeight: 12, seaLevel: 65, signs: true },
  { name: 'Regional Overview', scale: 3.0, buildingHeight: 6, seaLevel: 65, signs: false },
  { name: 'Natural Landscape', scale: 1.0, buildingHeight: 4, seaLevel: 65, signs: false },
];
