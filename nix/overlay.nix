{
  crane,
  fenix,
}: final: prev: let
  sss = prev.callPackage ./. {inherit crane fenix;};
in {
  sss = sss.packages.default;
  sssCode = sss.packages.code;
}
