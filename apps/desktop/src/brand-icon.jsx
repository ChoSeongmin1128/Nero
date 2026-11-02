import * as React from 'react';
import { brandIconUrl } from './brand-icons.js';

export function BrandIcon({ name }) {
  return <img className="brand-icon" src={brandIconUrl(name)} alt="" aria-hidden="true" />;
}
