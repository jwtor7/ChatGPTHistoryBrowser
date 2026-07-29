import brandArtwork from '../assets/app-icon.svg';

export function BrandMark({ className = '' }: { className?: string }) {
  return (
    <span className={`brand-mark${className ? ` ${className}` : ''}`} aria-hidden="true">
      <img src={brandArtwork} alt="" />
    </span>
  );
}

export function BrandArtwork({ className = '' }: { className?: string }) {
  return <img className={className} src={brandArtwork} alt="" aria-hidden="true" />;
}
