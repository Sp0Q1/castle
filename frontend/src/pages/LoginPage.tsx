import {
  type ActionFunctionArgs,
  Form,
  Link,
  redirect,
  useActionData,
  useNavigation,
} from "react-router-dom";
import { errorMessage } from "../api/client";
import { login } from "../auth/session";

export async function action({ request }: ActionFunctionArgs) {
  const form = await request.formData();
  try {
    await login(String(form.get("email")), String(form.get("password")));
  } catch (err) {
    return { error: errorMessage(err, "Login failed") };
  }
  return redirect("/");
}

function LoginPage() {
  const data = useActionData<{ error: string }>();
  const busy = useNavigation().state === "submitting";

  return (
    <div className="auth-card card">
      <h1>Sign in</h1>
      <p className="muted">Castle — security reporting portal</p>
      <Form method="post">
        <label>
          Email
          <input type="email" name="email" autoComplete="username" required />
        </label>
        <label>
          Password
          <input
            type="password"
            name="password"
            autoComplete="current-password"
            required
          />
        </label>
        {data?.error && <div className="alert">{data.error}</div>}
        <button type="submit" className="btn btn-primary" disabled={busy}>
          {busy ? "Signing in…" : "Sign in"}
        </button>
      </Form>
      <p className="muted">
        No account? <Link to="/register">Register</Link>
      </p>
    </div>
  );
}

export { LoginPage as Component };
