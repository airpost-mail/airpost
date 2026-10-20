# Darkside
**An dark sidebar theme for [Mailspring](https://getmailspring.com). Created by [Jamie Wilson](http://jamiewilson.io)**

#### Customize

> **Open the theme directory**  
> If you're on a Mac, you can find the theme files at `/Library/Application Support/Mailspring/packages`. To get there quickly, use the key command <kbd>Cmd</kbd> + <kbd>Shift</kbd> + <kbd>G</kbd> and enter `/Library/Application Support/Mailspring/packages`.

> **Change package.json**  
> In order to avoid conflicts between your custom theme and the pre-installed version, change `name` and `displayName` in `package.json` to:

    "name": "ui-darkside-custom",
    "displayName": "Darkside Custom",

> **Edit the colours**  
> Open `styles/tokens.css`. The whole theme follows two colours: `--sidebar-background` and `--primary`. Replace them with one of the palettes listed at the top of the file, or your own. Update `styles/theme-colors.less` to match so the theme preview shows your colours.

```css
:root {
  --primary: #f18260;
  --sidebar-background: #15171d;
}
```

### Feedback
If you have questions or suggestions, please submit an issue. If you need to, you can email me at [jamie@jamiewilson.io](mailto:jamie@jamiewilson?subject=Re: Darkside).
