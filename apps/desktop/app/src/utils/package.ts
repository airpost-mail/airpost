import pkg from '../../package.json';

pkg.desktopName =
  pkg.desktopName || (pkg.name ? `${pkg.name}.desktop` : 'com.ternmail.tern.desktop');

export default pkg;
