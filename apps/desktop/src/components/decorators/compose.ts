// Decorators wrap the component in classes whose types the callers don't spell out, and
// callers render the result with the base component's props plus the decorators' own. This
// keeps the signature that was implicitly any before strict mode.
export default function compose(
  BaseComponent: any,
  ...decorators: Array<(component: any) => any>
): any {
  const ComposedComponent = decorators.reduce((comp, decorator) => decorator(comp), BaseComponent);
  ComposedComponent.displayName = BaseComponent.displayName;
  ComposedComponent.containerRequired = BaseComponent.containerRequired;
  ComposedComponent.containerStyles = BaseComponent.containerStyles;
  return ComposedComponent;
}
