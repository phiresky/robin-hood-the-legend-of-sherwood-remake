export interface BC7FModule {
  HEAPU8: Uint8Array;
  _initialize(): void;
  _malloc(bytes: number): number;
  _free(pointer: number): void;
  _encode(source: number, width: number, height: number, output: number, level: number): void;
  _mip(source: number, width: number, height: number, output: number): void;
}
export default function createBC7F(options: {
  locateFile: (name: string) => string;
}): Promise<BC7FModule>;
