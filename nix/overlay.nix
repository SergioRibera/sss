{
  crane,
  cranix,
  fenix,
}: final: prev: let
  sss = prev.callPackage ./. {inherit crane cranix fenix;};
in {
  sss = sss.packages.default;
  sssCode = sss.packages.code;
}
