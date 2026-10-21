export interface LinkTrackingClick {
  recipient: string;
  timestamp: number;
}

/** One tracked link of a sent message, as the tracking server reports it. */
export interface LinkTrackingLink {
  url: string;
  redirect_url: string;
  click_count: number;
  click_data: LinkTrackingClick[];
}

export interface LinkTrackingMetadata {
  uid: string;
  tracked?: boolean;
  links?: LinkTrackingLink[];
}
