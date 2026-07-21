export type Role = "manager" | "staff" | "client";

/** GET /api/auth/current */
export interface CurrentUser {
  id: number;
  pid: string;
  name: string;
  email: string;
  role: Role;
  /** Proxy mode only: where to send the browser on sign-out. */
  logout_url?: string | null;
}

/** POST /api/auth/login */
export interface LoginResponse {
  token: string;
  pid: string;
  name: string;
  is_verified: boolean;
}

export interface UserSummary {
  id: number;
  pid: string;
  name: string;
  email: string;
  role: string;
}

export interface Project {
  id: number;
  pid: string;
  name: string;
  description: string | null;
  status: string;
  created_by: number;
  created_at: string;
  updated_at: string;
}

export interface Member {
  id: number;
  /** capacity onboarded in: "staff" | "client" */
  role: string;
  user: UserSummary;
  created_at: string;
}

export interface Finding {
  id: number;
  pid: string;
  project_id: number;
  author_id: number;
  title: string;
  finding_type: string;
  description: string;
  technical_description: string;
  impact: string;
  recommendation: string;
  severity: string;
  /** "draft" | "published" */
  status: string;
  created_at: string;
  updated_at: string;
}

export interface Comment {
  id: number;
  body: string;
  author: UserSummary;
  created_at: string;
}

/** GET /api/findings/{id} — a Finding plus its author and comment thread */
export interface FindingDetail extends Finding {
  author: UserSummary;
  comments: Comment[];
}

export interface CreateFindingParams {
  title: string;
  finding_type?: string;
  description: string;
  technical_description: string;
  impact: string;
  recommendation: string;
  severity?: string;
}

export interface UpdateFindingParams {
  title?: string;
  finding_type?: string;
  description?: string;
  technical_description?: string;
  impact?: string;
  recommendation?: string;
  severity?: string;
}

export const SEVERITIES = ["low", "medium", "elevated", "high", "extreme"] as const;

