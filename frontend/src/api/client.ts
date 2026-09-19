import type {
  AuthMode,
  Comment,
  CreateFindingParams,
  CurrentUser,
  Finding,
  FindingDetail,
  LoginResponse,
  Member,
  Project,
  UpdateFindingParams,
} from "./types";

/** Error carrying the HTTP status and the message the API returned. */
export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
    this.name = "ApiError";
  }
}

/** Both `catch` bindings and route errors are `unknown`, and both end up on screen. */
export function errorMessage(err: unknown, fallback = "Something went wrong") {
  return err instanceof Error ? err.message : fallback;
}

// The JWT is held here and set by the auth layer, so callers never pass it — and
// loaders and actions, which run outside React, can reach it.
let authToken: string | null = null;
export function setAuthToken(token: string | null): void {
  authToken = token;
}

const BASE = "/api";

type Options = Omit<RequestInit, "headers"> & {
  headers?: Record<string, string>;
};

// Attaches the bearer token (jwt mode — in proxy mode identity rides on the
// headers oauth2-proxy sets) and turns any non-2xx into an ApiError.
async function send(url: string, options: Options = {}): Promise<Response> {
  const headers: Record<string, string> = { ...options.headers };
  if (authToken) {
    headers.authorization = `Bearer ${authToken}`;
  }
  const res = await fetch(url, {
    ...options,
    headers,
    credentials: "same-origin",
  });
  if (!res.ok) {
    const body = await res.json().catch(() => null);
    throw new ApiError(
      res.status,
      body?.description || body?.error || res.statusText || "Request failed",
    );
  }
  return res;
}

async function request<T>(path: string, options: Options = {}): Promise<T> {
  const res = await send(`${BASE}${path}`, {
    ...options,
    headers: { "content-type": "application/json", ...options.headers },
  });
  const text = await res.text();
  return (text ? JSON.parse(text) : null) as T;
}

export const api = {
  register: (email: string, password: string, name: string): Promise<unknown> =>
    request("/auth/register", {
      method: "POST",
      body: JSON.stringify({ email, password, name }),
    }),

  login: (email: string, password: string): Promise<LoginResponse> =>
    request<LoginResponse>("/auth/login", {
      method: "POST",
      body: JSON.stringify({ email, password }),
    }),

  currentUser: (): Promise<CurrentUser> =>
    request<CurrentUser>("/auth/current"),

  authMode: (): Promise<{ mode: AuthMode }> =>
    request<{ mode: AuthMode }>("/auth/mode"),

  listProjects: (): Promise<Project[]> => request<Project[]>("/projects"),

  getProject: (id: number): Promise<Project> =>
    request<Project>(`/projects/${id}`),

  createProject: (name: string, description: string | null): Promise<Project> =>
    request<Project>("/projects", {
      method: "POST",
      body: JSON.stringify({ name, description }),
    }),

  listMembers: (projectId: number): Promise<Member[]> =>
    request<Member[]>(`/projects/${projectId}/members`),

  onboard: (
    projectId: number,
    userEmail: string,
    role: string,
  ): Promise<Member> =>
    request<Member>(`/projects/${projectId}/members`, {
      method: "POST",
      body: JSON.stringify({ user_email: userEmail, role }),
    }),

  listFindings: (projectId: number): Promise<Finding[]> =>
    request<Finding[]>(`/projects/${projectId}/findings`),

  createFinding: (
    projectId: number,
    params: CreateFindingParams,
  ): Promise<Finding> =>
    request<Finding>(`/projects/${projectId}/findings`, {
      method: "POST",
      body: JSON.stringify(params),
    }),

  updateFinding: (id: number, params: UpdateFindingParams): Promise<Finding> =>
    request<Finding>(`/findings/${id}`, {
      method: "PUT",
      body: JSON.stringify(params),
    }),

  deleteFinding: (id: number): Promise<void> =>
    request<void>(`/findings/${id}`, { method: "DELETE" }),

  getFinding: (id: number): Promise<FindingDetail> =>
    request<FindingDetail>(`/findings/${id}`),

  publishFinding: (id: number): Promise<Finding> =>
    request<Finding>(`/findings/${id}/publish`, { method: "POST" }),

  unpublishFinding: (id: number): Promise<Finding> =>
    request<Finding>(`/findings/${id}/unpublish`, { method: "POST" }),

  listComments: (findingId: number): Promise<Comment[]> =>
    request<Comment[]>(`/findings/${findingId}/comments`),

  addComment: (findingId: number, body: string): Promise<Comment> =>
    request<Comment>(`/findings/${findingId}/comments`, {
      method: "POST",
      body: JSON.stringify({ body }),
    }),

  // Uploads a single image (multipart) and returns the URL to embed in markdown.
  // Not `request`, because FormData must set its own multipart content-type.
  uploadImage: async (file: File): Promise<{ url: string }> => {
    const form = new FormData();
    form.append("file", file);
    const res = await send(`${BASE}/uploads`, { method: "POST", body: form });
    return res.json();
  },

  /**
   * Fetches an uploaded image as a Blob.
   *
   * Uploads are auth-gated server-side, so a plain `<img src="/api/uploads/…">`
   * fails: the browser issues that request with no Authorization header and the
   * JWT lives in sessionStorage, not a cookie. Images are therefore fetched
   * here — with the header in jwt mode, with the session cookie in proxy mode —
   * and rendered from an object URL (the CSP allows `img-src blob:`).
   */
  fetchUpload: async (path: string): Promise<Blob> => (await send(path)).blob(),
};
