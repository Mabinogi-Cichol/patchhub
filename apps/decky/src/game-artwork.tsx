import { ErrorBoundary, findModuleExport } from "@decky/ui";
import { useRef, useState, type Component, type ComponentClass, type ComponentType, type ReactNode } from "react";

type Overview = NonNullable<ReturnType<typeof window.appStore.GetAppOverviewByAppID>>;
// Steam ELibraryAssetType.Hero (declared in @decky/ui's App.ts, not re-exported).
const HERO_ASSET_TYPE = 1;
type NativeImageProps = {
  app: Overview;
  eAssetType: number;
  className: string;
  imageClassName: string;
  allowCustomization: boolean;
  neverShowTitle: boolean;
  suppressTransitions: boolean;
  backgroundType: string;
  alt: string;
  onLoad: () => void;
  onError: () => void;
};
type NativeImageInstance = Component<NativeImageProps> & { readonly is_default_image: boolean };

// Steam's LibraryAssetImage owns artwork selection, custom images and caching.
// Find it by its methods, not the current build's minified module/export names.
const NativeImage = findModuleExport(value =>
  typeof value === "function" && typeof value.prototype?.GetSourcesForAsset === "function"
    && typeof value.prototype?.OnImageLoaded === "function",
) as ComponentClass<NativeImageProps> & (new (props: NativeImageProps) => NativeImageInstance) | undefined;

// The native boundary supports fallback/errorKey; @decky/ui only types children.
const ArtworkBoundary = ErrorBoundary as ComponentType<{ children: ReactNode; fallback: ReactNode; errorKey: number }>;

function ArtworkImage({ app }: { app: Overview }) {
  const image = useRef<NativeImageInstance>(null);
  const [loaded, setLoaded] = useState(false);
  const [failed, setFailed] = useState(false);
  if (!NativeImage || failed) return null;
  return <div className="patch-game-artwork" hidden={!loaded}>
    <NativeImage ref={image} app={app} eAssetType={HERO_ASSET_TYPE}
      className="patch-game-artwork-image" imageClassName="patch-game-artwork-content"
      allowCustomization={false} neverShowTitle suppressTransitions backgroundType="transparent" alt=""
      onLoad={() => {
        // Steam's final source is its generic default Hero, not this game's art.
        if (image.current?.is_default_image) setFailed(true);
        else setLoaded(true);
      }} onError={() => setFailed(true)} />
  </div>;
}

export function GameArtwork({ appid }: { appid: number }) {
  let app: Overview | null;
  try { app = window.appStore?.GetAppOverviewByAppID(appid); }
  catch { return null; } // Artwork is optional at the Steam client boundary.
  if (!app || !NativeImage) return null;
  return <ArtworkBoundary key={appid} errorKey={appid} fallback={null}>
    <ArtworkImage app={app} />
  </ArtworkBoundary>;
}
