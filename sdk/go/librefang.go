/*
LibreFang Go SDK — AUTO-GENERATED from openapi.json.
Do not edit manually. Run: python3 scripts/codegen-sdks.py
*/
package librefang

import (
	"bufio"
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
)

// LibreFangError represents an API error.
type LibreFangError struct {
	Message string
	Status  int
	Body    string
}

func (e *LibreFangError) Error() string {
	return fmt.Sprintf("HTTP %d: %s", e.Status, e.Message)
}

// Client is the LibreFang REST API client.
type Client struct {
	BaseURL string
	Headers map[string]string
	HTTP    *http.Client

	A2A *A2AResource
	Agents *AgentsResource
	Approvals *ApprovalsResource
	Auth *AuthResource
	AutoDream *AutoDreamResource
	Budget *BudgetResource
	Channels *ChannelsResource
	Extensions *ExtensionsResource
	Goals *GoalsResource
	Groups *GroupsResource
	Hands *HandsResource
	Inbox *InboxResource
	Knowledge *KnowledgeResource
	Mcp *McpResource
	Media *MediaResource
	Memory *MemoryResource
	Models *ModelsResource
	Network *NetworkResource
	Pairing *PairingResource
	Plugins *PluginsResource
	ProactiveMemory *ProactiveMemoryResource
	Sessions *SessionsResource
	Skills *SkillsResource
	System *SystemResource
	Tasks *TasksResource
	Tools *ToolsResource
	Users *UsersResource
	Vault *VaultResource
	Webhooks *WebhooksResource
	Workflows *WorkflowsResource
}

// New creates a new LibreFang client.
func New(baseURL string) *Client {
	baseURL = strings.TrimSuffix(baseURL, "/")
	c := &Client{
		BaseURL: baseURL,
		Headers: map[string]string{"Content-Type": "application/json"},
		HTTP:    &http.Client{},
	}
		c.A2A = &A2AResource{client: c}
		c.Agents = &AgentsResource{client: c}
		c.Approvals = &ApprovalsResource{client: c}
		c.Auth = &AuthResource{client: c}
		c.AutoDream = &AutoDreamResource{client: c}
		c.Budget = &BudgetResource{client: c}
		c.Channels = &ChannelsResource{client: c}
		c.Extensions = &ExtensionsResource{client: c}
		c.Goals = &GoalsResource{client: c}
		c.Groups = &GroupsResource{client: c}
		c.Hands = &HandsResource{client: c}
		c.Inbox = &InboxResource{client: c}
		c.Knowledge = &KnowledgeResource{client: c}
		c.Mcp = &McpResource{client: c}
		c.Media = &MediaResource{client: c}
		c.Memory = &MemoryResource{client: c}
		c.Models = &ModelsResource{client: c}
		c.Network = &NetworkResource{client: c}
		c.Pairing = &PairingResource{client: c}
		c.Plugins = &PluginsResource{client: c}
		c.ProactiveMemory = &ProactiveMemoryResource{client: c}
		c.Sessions = &SessionsResource{client: c}
		c.Skills = &SkillsResource{client: c}
		c.System = &SystemResource{client: c}
		c.Tasks = &TasksResource{client: c}
		c.Tools = &ToolsResource{client: c}
		c.Users = &UsersResource{client: c}
		c.Vault = &VaultResource{client: c}
		c.Webhooks = &WebhooksResource{client: c}
		c.Workflows = &WorkflowsResource{client: c}
	return c
}

func (c *Client) withQuery(path string, query map[string]string) string {
	if len(query) == 0 {
		return path
	}
	vals := url.Values{}
	for k, v := range query {
		if v == "" {
			continue
		}
		vals.Set(k, v)
	}
	q := vals.Encode()
	if q == "" {
		return path
	}
	if strings.Contains(path, "?") {
		return path + "&" + q
	}
	return path + "?" + q
}

func (c *Client) request(method, path string, body interface{}, query map[string]string) (interface{}, error) {
	var bodyBytes []byte
	if body != nil {
		b, err := json.Marshal(body)
		if err != nil {
			return nil, fmt.Errorf("marshal: %w", err)
		}
		bodyBytes = b
	}
	return c.do(method, c.withQuery(path, query), bodyBytes, "")
}

// requestRaw sends body verbatim under contentType, for endpoints that read
// the request body as bytes and reject application/json.
func (c *Client) requestRaw(method, path string, body []byte, contentType string) (interface{}, error) {
	return c.do(method, path, body, contentType)
}

func (c *Client) do(method, path string, bodyBytes []byte, contentType string) (interface{}, error) {
	urlStr := c.BaseURL + path
	req, err := http.NewRequest(method, urlStr, bytes.NewReader(bodyBytes))
	if err != nil {
		return nil, err
	}
	for k, v := range c.Headers {
		req.Header.Set(k, v)
	}
	if contentType != "" {
		req.Header.Set("Content-Type", contentType)
	}
	resp, err := c.HTTP.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	respBody, _ := io.ReadAll(resp.Body)
	if resp.StatusCode >= 400 {
		return nil, &LibreFangError{Message: string(respBody), Status: resp.StatusCode, Body: string(respBody)}
	}
	var arr []json.RawMessage
	if err := json.Unmarshal(respBody, &arr); err == nil {
		return arr, nil
	}
	var result map[string]interface{}
	if err := json.Unmarshal(respBody, &result); err != nil {
		return string(respBody), nil
	}
	return result, nil
}

func (c *Client) stream(method, path string, body interface{}, query map[string]string) <-chan map[string]interface{} {
	ch := make(chan map[string]interface{})
	go func() {
		defer close(ch)
		urlStr := c.BaseURL + c.withQuery(path, query)
		var bodyBytes []byte
		if body != nil {
			b, err := json.Marshal(body)
			if err != nil {
				ch <- map[string]interface{}{"error": fmt.Sprintf("marshal: %v", err), "status": 0}
				return
			}
			bodyBytes = b
		}
		req, err := http.NewRequest(method, urlStr, bytes.NewReader(bodyBytes))
		if err != nil {
			ch <- map[string]interface{}{"error": fmt.Sprintf("new request: %v", err), "status": 0}
			return
		}
		for k, v := range c.Headers {
			req.Header.Set(k, v)
		}
		req.Header.Set("Accept", "text/event-stream")
		resp, err := c.HTTP.Do(req)
		if err != nil {
			ch <- map[string]interface{}{"error": err.Error(), "status": 0}
			return
		}
		defer resp.Body.Close()
		if resp.StatusCode >= 400 {
			body, _ := io.ReadAll(resp.Body)
			ch <- map[string]interface{}{
				"error":  fmt.Sprintf("HTTP %d: %s", resp.StatusCode, string(body)),
				"status": resp.StatusCode,
			}
			return
		}
		// Accumulate partial lines across reads; SSE events can span chunks.
		// bufio.Reader grows its internal buffer without bound on unterminated
		// input; a limited reader plus explicit size checks cap memory use.
		const maxSSELine = 8 * 1024 * 1024
		reader := bufio.NewReaderSize(resp.Body, 64*1024)
		for {
			line, err := reader.ReadString('\n')
			if len(line) > maxSSELine {
				ch <- map[string]interface{}{
					"error":  fmt.Sprintf("SSE line exceeded %d bytes", maxSSELine),
					"status": 0,
				}
				return
			}
			if line != "" {
				trimmed := strings.TrimSpace(line)
				if strings.HasPrefix(trimmed, "data: ") {
					data := strings.TrimPrefix(trimmed, "data: ")
					if data == "[DONE]" {
						return
					}
					var event map[string]interface{}
					if jerr := json.Unmarshal([]byte(data), &event); jerr != nil {
						ch <- map[string]interface{}{"raw": data}
					} else {
						ch <- event
					}
				}
			}
			if err != nil {
				return
			}
		}
	}()
	return ch
}

// ToMap converts an interface{} to map[string]interface{}.
func ToMap(v interface{}) map[string]interface{} {
	if m, ok := v.(map[string]interface{}); ok {
		return m
	}
	return map[string]interface{}{}
}

// ToSlice converts an interface{} to []map[string]interface{}.
func ToSlice(v interface{}) []map[string]interface{} {
	switch t := v.(type) {
	case []json.RawMessage:
		out := make([]map[string]interface{}, len(t))
		for i, raw := range t {
			json.Unmarshal(raw, &out[i])
		}
		return out
	case []interface{}:
		out := make([]map[string]interface{}, len(t))
		for i, a := range t {
			if m, ok := a.(map[string]interface{}); ok {
				out[i] = m
			}
		}
		return out
	}
	return nil
}

// ── A2A Resource

type A2AResource struct{ client *Client }

func (r *A2AResource) A2AListExternalAgents() (interface{}, error) {
	return r.client.request("GET", "/api/a2a/agents", nil, nil)
}

func (r *A2AResource) A2AGetExternalAgent(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/a2a/agents/%s", url.PathEscape(id)), nil, nil)
}

func (r *A2AResource) A2AApproveExternal(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/a2a/agents/%s/approve", url.PathEscape(id)), nil, nil)
}

func (r *A2AResource) A2ADiscoverExternal(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/a2a/discover", data, nil)
}

func (r *A2AResource) A2ASendExternal(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/a2a/send", data, nil)
}

func (r *A2AResource) A2AExternalTaskStatus(id string, query map[string]string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/a2a/tasks/%s/status", url.PathEscape(id)), nil, query)
}

// ── Agents Resource

type AgentsResource struct{ client *Client }

func (r *AgentsResource) ListAgents(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/agents", nil, query)
}

func (r *AgentsResource) SpawnAgent(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/agents", data, nil)
}

func (r *AgentsResource) BulkCreateAgents(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/agents/bulk", data, nil)
}

func (r *AgentsResource) BulkDeleteAgents() (interface{}, error) {
	return r.client.request("DELETE", "/api/agents/bulk", nil, nil)
}

func (r *AgentsResource) BulkStartAgents(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/agents/bulk/start", data, nil)
}

func (r *AgentsResource) BulkStopAgents(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/agents/bulk/stop", data, nil)
}

func (r *AgentsResource) ListAgentIdentities() (interface{}, error) {
	return r.client.request("GET", "/api/agents/identities", nil, nil)
}

func (r *AgentsResource) ResetAgentIdentity(name string, query map[string]string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/identities/%s/reset", url.PathEscape(name)), nil, query)
}

func (r *AgentsResource) SpawnEphemeralAgent(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/agents/spawn-ephemeral", data, nil)
}

func (r *AgentsResource) GetAgent(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s", url.PathEscape(id)), nil, nil)
}

func (r *AgentsResource) KillAgent(id string, query map[string]string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/agents/%s", url.PathEscape(id)), nil, query)
}

func (r *AgentsResource) PatchAgent(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PATCH", fmt.Sprintf("/api/agents/%s", url.PathEscape(id)), data, nil)
}

func (r *AgentsResource) ServeAgentAvatar(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/avatar", url.PathEscape(id)), nil, nil)
}

// UploadAgentAvatar sends a raw application/octet-stream body. An empty contentType defaults to it.
func (r *AgentsResource) UploadAgentAvatar(id string, body []byte, contentType string) (interface{}, error) {
	if contentType == "" {
		contentType = "application/octet-stream"
	}
	return r.client.requestRaw("POST", fmt.Sprintf("/api/agents/%s/avatar", url.PathEscape(id)), body, contentType)
}

func (r *AgentsResource) DeleteAgentAvatar(id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/agents/%s/avatar", url.PathEscape(id)), nil, nil)
}

func (r *AgentsResource) GetAgentChannels(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/channels", url.PathEscape(id)), nil, nil)
}

func (r *AgentsResource) SetAgentChannels(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/agents/%s/channels", url.PathEscape(id)), data, nil)
}

func (r *AgentsResource) CloneAgent(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/clone", url.PathEscape(id)), data, nil)
}

func (r *AgentsResource) PatchAgentConfig(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PATCH", fmt.Sprintf("/api/agents/%s/config", url.PathEscape(id)), data, nil)
}

func (r *AgentsResource) GetAgentDeliveries(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/deliveries", url.PathEscape(id)), nil, nil)
}

func (r *AgentsResource) ListAgentEphemeralRuns(id string, query map[string]string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/ephemeral-runs", url.PathEscape(id)), nil, query)
}

func (r *AgentsResource) ListAgentEvents(id string, query map[string]string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/events", url.PathEscape(id)), nil, query)
}

func (r *AgentsResource) ListAgentFiles(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/files", url.PathEscape(id)), nil, nil)
}

func (r *AgentsResource) GetAgentFile(id string, filename string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/files/%s", url.PathEscape(id), url.PathEscape(filename)), nil, nil)
}

func (r *AgentsResource) SetAgentFile(id string, filename string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/agents/%s/files/%s", url.PathEscape(id), url.PathEscape(filename)), data, nil)
}

func (r *AgentsResource) DeleteAgentFile(id string, filename string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/agents/%s/files/%s", url.PathEscape(id), url.PathEscape(filename)), nil, nil)
}

func (r *AgentsResource) DeleteHandAgentRuntimeConfig(id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/agents/%s/hand-runtime-config", url.PathEscape(id)), nil, nil)
}

func (r *AgentsResource) PatchHandAgentRuntimeConfig(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PATCH", fmt.Sprintf("/api/agents/%s/hand-runtime-config", url.PathEscape(id)), data, nil)
}

func (r *AgentsResource) ClearAgentHistory(id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/agents/%s/history", url.PathEscape(id)), nil, nil)
}

func (r *AgentsResource) UpdateAgentIdentity(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PATCH", fmt.Sprintf("/api/agents/%s/identity", url.PathEscape(id)), data, nil)
}

func (r *AgentsResource) InjectMessage(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/inject", url.PathEscape(id)), data, nil)
}

func (r *AgentsResource) AgentLogs(id string, query map[string]string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/logs", url.PathEscape(id)), nil, query)
}

func (r *AgentsResource) GetAgentManifestToml(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/manifest", url.PathEscape(id)), nil, nil)
}

func (r *AgentsResource) GetAgentMcpServers(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/mcp_servers", url.PathEscape(id)), nil, nil)
}

func (r *AgentsResource) SetAgentMcpServers(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/agents/%s/mcp_servers", url.PathEscape(id)), data, nil)
}

func (r *AgentsResource) SendMessage(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/message", url.PathEscape(id)), data, nil)
}

func (r *AgentsResource) SendMessageStream(id string, data map[string]interface{}) <-chan map[string]interface{} {
	return r.client.stream("POST", fmt.Sprintf("/api/agents/%s/message/stream", url.PathEscape(id)), data, nil)
}

func (r *AgentsResource) AgentMetrics(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/metrics", url.PathEscape(id)), nil, nil)
}

func (r *AgentsResource) SetAgentMode(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/agents/%s/mode", url.PathEscape(id)), data, nil)
}

func (r *AgentsResource) SetModel(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/agents/%s/model", url.PathEscape(id)), data, nil)
}

func (r *AgentsResource) GetAgentModelRouting(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/model_routing", url.PathEscape(id)), nil, nil)
}

func (r *AgentsResource) SetAgentModelRouting(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/agents/%s/model_routing", url.PathEscape(id)), data, nil)
}

func (r *AgentsResource) PushMessage(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/push", url.PathEscape(id)), data, nil)
}

func (r *AgentsResource) ReloadAgentManifest(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/reload", url.PathEscape(id)), nil, nil)
}

func (r *AgentsResource) ResumeAgent(id string) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/agents/%s/resume", url.PathEscape(id)), nil, nil)
}

func (r *AgentsResource) ListAgentRuntime(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/runtime", url.PathEscape(id)), nil, nil)
}

func (r *AgentsResource) SaveAgentAsAgentType(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/save-as-agent-type", url.PathEscape(id)), data, nil)
}

func (r *AgentsResource) GetAgentSession(id string, query map[string]string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/session", url.PathEscape(id)), nil, query)
}

func (r *AgentsResource) CompactSession(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/session/compact", url.PathEscape(id)), nil, nil)
}

func (r *AgentsResource) GetAgentSessionContext(id string, query map[string]string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/session/context", url.PathEscape(id)), nil, query)
}

func (r *AgentsResource) RebootSession(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/session/reboot", url.PathEscape(id)), nil, nil)
}

func (r *AgentsResource) ResetSession(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/session/reset", url.PathEscape(id)), nil, nil)
}

func (r *AgentsResource) ListAgentSessions(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/sessions", url.PathEscape(id)), nil, nil)
}

func (r *AgentsResource) CreateAgentSession(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/sessions", url.PathEscape(id)), data, nil)
}

func (r *AgentsResource) ImportSession(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/sessions/import", url.PathEscape(id)), data, nil)
}

func (r *AgentsResource) ExportSession(id string, session_id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/sessions/%s/export", url.PathEscape(id), url.PathEscape(session_id)), nil, nil)
}

func (r *AgentsResource) StopSession(id string, session_id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/sessions/%s/stop", url.PathEscape(id), url.PathEscape(session_id)), nil, nil)
}

func (r *AgentsResource) AttachSessionStream(id string, session_id string) <-chan map[string]interface{} {
	return r.client.stream("GET", fmt.Sprintf("/api/agents/%s/sessions/%s/stream", url.PathEscape(id), url.PathEscape(session_id)), nil, nil)
}

func (r *AgentsResource) SwitchAgentSession(id string, session_id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/sessions/%s/switch", url.PathEscape(id), url.PathEscape(session_id)), nil, nil)
}

func (r *AgentsResource) ExportSessionTrajectory(id string, session_id string, query map[string]string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/sessions/%s/trajectory", url.PathEscape(id), url.PathEscape(session_id)), nil, query)
}

func (r *AgentsResource) GetAgentSkills(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/skills", url.PathEscape(id)), nil, nil)
}

func (r *AgentsResource) SetAgentSkills(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/agents/%s/skills", url.PathEscape(id)), data, nil)
}

func (r *AgentsResource) GetAgentStats(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/stats", url.PathEscape(id)), nil, nil)
}

func (r *AgentsResource) StopAgent(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/stop", url.PathEscape(id)), nil, nil)
}

func (r *AgentsResource) SuspendAgent(id string) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/agents/%s/suspend", url.PathEscape(id)), nil, nil)
}

func (r *AgentsResource) GetAgentTools(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/tools", url.PathEscape(id)), nil, nil)
}

func (r *AgentsResource) SetAgentTools(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/agents/%s/tools", url.PathEscape(id)), data, nil)
}

func (r *AgentsResource) GetAgentTraces(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/traces", url.PathEscape(id)), nil, nil)
}

// UploadFile sends a raw application/octet-stream body. An empty contentType defaults to it.
func (r *AgentsResource) UploadFile(id string, body []byte, contentType string) (interface{}, error) {
	if contentType == "" {
		contentType = "application/octet-stream"
	}
	return r.client.requestRaw("POST", fmt.Sprintf("/api/agents/%s/upload", url.PathEscape(id)), body, contentType)
}

func (r *AgentsResource) ServeUpload(file_id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/uploads/%s", url.PathEscape(file_id)), nil, nil)
}

// ── Approvals Resource

type ApprovalsResource struct{ client *Client }

func (r *ApprovalsResource) ListApprovals(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/approvals", nil, query)
}

func (r *ApprovalsResource) CreateApproval(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/approvals", data, nil)
}

func (r *ApprovalsResource) AuditLog(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/approvals/audit", nil, query)
}

func (r *ApprovalsResource) BatchResolve(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/approvals/batch", data, nil)
}

func (r *ApprovalsResource) ApprovalCount() (interface{}, error) {
	return r.client.request("GET", "/api/approvals/count", nil, nil)
}

func (r *ApprovalsResource) ListApprovalsForSession(session_id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/approvals/session/%s", url.PathEscape(session_id)), nil, nil)
}

func (r *ApprovalsResource) ApproveAllForSession(session_id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/approvals/session/%s/approve_all", url.PathEscape(session_id)), data, nil)
}

func (r *ApprovalsResource) RejectAllForSession(session_id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/approvals/session/%s/reject_all", url.PathEscape(session_id)), nil, nil)
}

func (r *ApprovalsResource) GetApproval(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/approvals/%s", url.PathEscape(id)), nil, nil)
}

func (r *ApprovalsResource) ApproveRequest(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/approvals/%s/approve", url.PathEscape(id)), data, nil)
}

func (r *ApprovalsResource) ModifyRequest(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/approvals/%s/modify", url.PathEscape(id)), data, nil)
}

func (r *ApprovalsResource) RejectRequest(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/approvals/%s/reject", url.PathEscape(id)), nil, nil)
}

// ── Auth Resource

type AuthResource struct{ client *Client }

func (r *AuthResource) AuthCallback() (interface{}, error) {
	return r.client.request("GET", "/api/auth/callback", nil, nil)
}

func (r *AuthResource) AuthCallbackPost(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/auth/callback", data, nil)
}

func (r *AuthResource) ChangePassword(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/auth/change-password", data, nil)
}

func (r *AuthResource) DashboardAuthCheck() (interface{}, error) {
	return r.client.request("GET", "/api/auth/dashboard-check", nil, nil)
}

func (r *AuthResource) DashboardLogin(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/auth/dashboard-login", data, nil)
}

func (r *AuthResource) AuthIntrospect(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/auth/introspect", data, nil)
}

func (r *AuthResource) AuthLogin() (interface{}, error) {
	return r.client.request("GET", "/api/auth/login", nil, nil)
}

func (r *AuthResource) AuthLoginProvider(provider string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/auth/login/%s", url.PathEscape(provider)), nil, nil)
}

func (r *AuthResource) DashboardLogout() (interface{}, error) {
	return r.client.request("POST", "/api/auth/logout", nil, nil)
}

func (r *AuthResource) AuthenticationOptions(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/auth/passkey/authentication-options", data, nil)
}

func (r *AuthResource) AuthenticationVerify(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/auth/passkey/authentication-verify", data, nil)
}

func (r *AuthResource) ListCredentials() (interface{}, error) {
	return r.client.request("GET", "/api/auth/passkey/credentials", nil, nil)
}

func (r *AuthResource) RevokeCredential(id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/auth/passkey/credentials/%s", url.PathEscape(id)), nil, nil)
}

func (r *AuthResource) RegistrationOptions(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/auth/passkey/registration-options", data, nil)
}

func (r *AuthResource) RegistrationVerify(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/auth/passkey/registration-verify", data, nil)
}

func (r *AuthResource) AuthProviders() (interface{}, error) {
	return r.client.request("GET", "/api/auth/providers", nil, nil)
}

func (r *AuthResource) AuthRefresh(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/auth/refresh", data, nil)
}

func (r *AuthResource) AuthUserinfo() (interface{}, error) {
	return r.client.request("GET", "/api/auth/userinfo", nil, nil)
}

// ── AutoDream Resource

type AutoDreamResource struct{ client *Client }

func (r *AutoDreamResource) AutoDreamAbort(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/auto-dream/agents/%s/abort", url.PathEscape(id)), nil, nil)
}

func (r *AutoDreamResource) AutoDreamSetEnabled(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/auto-dream/agents/%s/enabled", url.PathEscape(id)), data, nil)
}

func (r *AutoDreamResource) AutoDreamTrigger(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/auto-dream/agents/%s/trigger", url.PathEscape(id)), nil, nil)
}

func (r *AutoDreamResource) AutoDreamStatus() (interface{}, error) {
	return r.client.request("GET", "/api/auto-dream/status", nil, nil)
}

// ── Budget Resource

type BudgetResource struct{ client *Client }

func (r *BudgetResource) BudgetStatus() (interface{}, error) {
	return r.client.request("GET", "/api/budget", nil, nil)
}

func (r *BudgetResource) UpdateBudget(data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", "/api/budget", data, nil)
}

func (r *BudgetResource) AgentBudgetRanking() (interface{}, error) {
	return r.client.request("GET", "/api/budget/agents", nil, nil)
}

func (r *BudgetResource) AgentBudgetStatus(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/budget/agents/%s", url.PathEscape(id)), nil, nil)
}

func (r *BudgetResource) UpdateAgentBudget(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/budget/agents/%s", url.PathEscape(id)), data, nil)
}

func (r *BudgetResource) ProviderBudgetList() (interface{}, error) {
	return r.client.request("GET", "/api/budget/providers", nil, nil)
}

func (r *BudgetResource) UpdateProviderBudget(provider_id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/budget/providers/%s", url.PathEscape(provider_id)), data, nil)
}

func (r *BudgetResource) UserBudgetRanking(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/budget/users", nil, query)
}

func (r *BudgetResource) UserBudgetDetail(user_id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/budget/users/%s", url.PathEscape(user_id)), nil, nil)
}

func (r *BudgetResource) UpdateUserBudget(user_id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/budget/users/%s", url.PathEscape(user_id)), data, nil)
}

func (r *BudgetResource) DeleteUserBudget(user_id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/budget/users/%s", url.PathEscape(user_id)), nil, nil)
}

func (r *BudgetResource) UsageStats(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/usage", nil, query)
}

func (r *BudgetResource) UsageByModel(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/usage/by-model", nil, query)
}

func (r *BudgetResource) UsageByModelPerformance(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/usage/by-model/performance", nil, query)
}

func (r *BudgetResource) UsageDaily(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/usage/daily", nil, query)
}

func (r *BudgetResource) UsageExport(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/usage/export", nil, query)
}

func (r *BudgetResource) UsageSummary(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/usage/summary", nil, query)
}

// ── Channels Resource

type ChannelsResource struct{ client *Client }

func (r *ChannelsResource) ListChannels() (interface{}, error) {
	return r.client.request("GET", "/api/channels", nil, nil)
}

func (r *ChannelsResource) ListChannelRegistry() (interface{}, error) {
	return r.client.request("GET", "/api/channels/registry", nil, nil)
}

func (r *ChannelsResource) ReloadChannels() (interface{}, error) {
	return r.client.request("POST", "/api/channels/reload", nil, nil)
}

func (r *ChannelsResource) DeleteSidecarChannel(name string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/channels/sidecar/%s", url.PathEscape(name)), nil, nil)
}

func (r *ChannelsResource) ConfigureSidecarChannel(name string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/channels/sidecar/%s/configure", url.PathEscape(name)), data, nil)
}

func (r *ChannelsResource) GetChannelQr(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/channels/%s/qr", url.PathEscape(name)), nil, nil)
}

// ── Extensions Resource

type ExtensionsResource struct{ client *Client }

func (r *ExtensionsResource) ListExtensions() (interface{}, error) {
	return r.client.request("GET", "/api/extensions", nil, nil)
}

func (r *ExtensionsResource) InstallExtension(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/extensions/install", data, nil)
}

func (r *ExtensionsResource) UninstallExtension(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/extensions/uninstall", data, nil)
}

func (r *ExtensionsResource) GetExtension(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/extensions/%s", url.PathEscape(name)), nil, nil)
}

// ── Goals Resource

type GoalsResource struct{ client *Client }

func (r *GoalsResource) ListGoalTemplates() (interface{}, error) {
	return r.client.request("GET", "/api/goals/templates", nil, nil)
}

// ── Groups Resource

type GroupsResource struct{ client *Client }

func (r *GroupsResource) ListGroups() (interface{}, error) {
	return r.client.request("GET", "/api/groups", nil, nil)
}

func (r *GroupsResource) CreateGroup(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/groups", data, nil)
}

func (r *GroupsResource) GetGroup(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/groups/%s", url.PathEscape(name)), nil, nil)
}

func (r *GroupsResource) UpdateGroup(name string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/groups/%s", url.PathEscape(name)), data, nil)
}

func (r *GroupsResource) DeleteGroup(name string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/groups/%s", url.PathEscape(name)), nil, nil)
}

func (r *GroupsResource) AddGroupMember(name string, user string) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/groups/%s/members/%s", url.PathEscape(name), url.PathEscape(user)), nil, nil)
}

func (r *GroupsResource) RemoveGroupMember(name string, user string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/groups/%s/members/%s", url.PathEscape(name), url.PathEscape(user)), nil, nil)
}

func (r *GroupsResource) UserGroups(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/users/%s/groups", url.PathEscape(name)), nil, nil)
}

// ── Hands Resource

type HandsResource struct{ client *Client }

func (r *HandsResource) ListHands() (interface{}, error) {
	return r.client.request("GET", "/api/hands", nil, nil)
}

func (r *HandsResource) ListActiveHands() (interface{}, error) {
	return r.client.request("GET", "/api/hands/active", nil, nil)
}

func (r *HandsResource) InstallHand(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/hands/install", data, nil)
}

func (r *HandsResource) DeactivateHand(id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/hands/instances/%s", url.PathEscape(id)), nil, nil)
}

func (r *HandsResource) HandInstanceBrowser(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/hands/instances/%s/browser", url.PathEscape(id)), nil, nil)
}

func (r *HandsResource) PauseHand(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/hands/instances/%s/pause", url.PathEscape(id)), nil, nil)
}

func (r *HandsResource) ResumeHand(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/hands/instances/%s/resume", url.PathEscape(id)), nil, nil)
}

func (r *HandsResource) HandStats(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/hands/instances/%s/stats", url.PathEscape(id)), nil, nil)
}

func (r *HandsResource) InstallHandFromMarketplace(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/hands/marketplace/install", data, nil)
}

func (r *HandsResource) ReloadHands() (interface{}, error) {
	return r.client.request("POST", "/api/hands/reload", nil, nil)
}

func (r *HandsResource) GetHand(hand_id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/hands/%s", url.PathEscape(hand_id)), nil, nil)
}

func (r *HandsResource) UninstallHand(hand_id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/hands/%s", url.PathEscape(hand_id)), nil, nil)
}

func (r *HandsResource) ActivateHand(hand_id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/hands/%s/activate", url.PathEscape(hand_id)), data, nil)
}

func (r *HandsResource) CheckHandDeps(hand_id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/hands/%s/check-deps", url.PathEscape(hand_id)), nil, nil)
}

func (r *HandsResource) InstallHandDeps(hand_id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/hands/%s/install-deps", url.PathEscape(hand_id)), nil, nil)
}

func (r *HandsResource) GetHandManifest(hand_id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/hands/%s/manifest", url.PathEscape(hand_id)), nil, nil)
}

func (r *HandsResource) UpdateHandManifest(hand_id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/hands/%s/manifest", url.PathEscape(hand_id)), data, nil)
}

func (r *HandsResource) SetHandSecret(hand_id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/hands/%s/secret", url.PathEscape(hand_id)), data, nil)
}

func (r *HandsResource) GetHandSettings(hand_id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/hands/%s/settings", url.PathEscape(hand_id)), nil, nil)
}

func (r *HandsResource) UpdateHandSettings(hand_id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/hands/%s/settings", url.PathEscape(hand_id)), data, nil)
}

// ── Inbox Resource

type InboxResource struct{ client *Client }

func (r *InboxResource) InboxStatus() (interface{}, error) {
	return r.client.request("GET", "/api/inbox/status", nil, nil)
}

// ── Knowledge Resource

type KnowledgeResource struct{ client *Client }

func (r *KnowledgeResource) ListBases() (interface{}, error) {
	return r.client.request("GET", "/api/knowledge", nil, nil)
}

func (r *KnowledgeResource) CreateBase(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/knowledge", data, nil)
}

func (r *KnowledgeResource) DeleteBase(name string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/knowledge/%s", url.PathEscape(name)), nil, nil)
}

func (r *KnowledgeResource) SetHolders(name string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/knowledge/%s/agents", url.PathEscape(name)), data, nil)
}

func (r *KnowledgeResource) ListDocuments(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/knowledge/%s/documents", url.PathEscape(name)), nil, nil)
}

// PutDocument sends a raw application/octet-stream body. An empty contentType defaults to it.
func (r *KnowledgeResource) PutDocument(name string, filename string, body []byte, contentType string) (interface{}, error) {
	if contentType == "" {
		contentType = "application/octet-stream"
	}
	return r.client.requestRaw("PUT", fmt.Sprintf("/api/knowledge/%s/documents/%s", url.PathEscape(name), url.PathEscape(filename)), body, contentType)
}

func (r *KnowledgeResource) DeleteDocument(name string, filename string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/knowledge/%s/documents/%s", url.PathEscape(name), url.PathEscape(filename)), nil, nil)
}

// ── Mcp Resource

type McpResource struct{ client *Client }

func (r *McpResource) ListMcpCatalog() (interface{}, error) {
	return r.client.request("GET", "/api/mcp/catalog", nil, nil)
}

func (r *McpResource) GetMcpCatalogEntry(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/mcp/catalog/%s", url.PathEscape(id)), nil, nil)
}

func (r *McpResource) McpHealthHandler() (interface{}, error) {
	return r.client.request("GET", "/api/mcp/health", nil, nil)
}

func (r *McpResource) ReloadMcpHandler() (interface{}, error) {
	return r.client.request("POST", "/api/mcp/reload", nil, nil)
}

func (r *McpResource) ListMcpServers() (interface{}, error) {
	return r.client.request("GET", "/api/mcp/servers", nil, nil)
}

func (r *McpResource) AddMcpServer(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/mcp/servers", data, nil)
}

func (r *McpResource) GetMcpServer(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/mcp/servers/%s", url.PathEscape(name)), nil, nil)
}

func (r *McpResource) UpdateMcpServer(name string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/mcp/servers/%s", url.PathEscape(name)), data, nil)
}

func (r *McpResource) DeleteMcpServer(name string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/mcp/servers/%s", url.PathEscape(name)), nil, nil)
}

func (r *McpResource) AuthRevoke(name string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/mcp/servers/%s/auth/revoke", url.PathEscape(name)), nil, nil)
}

func (r *McpResource) AuthStart(name string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/mcp/servers/%s/auth/start", url.PathEscape(name)), nil, nil)
}

func (r *McpResource) AuthStatus(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/mcp/servers/%s/auth/status", url.PathEscape(name)), nil, nil)
}

func (r *McpResource) ReconnectMcpServerHandler(name string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/mcp/servers/%s/reconnect", url.PathEscape(name)), nil, nil)
}

func (r *McpResource) PatchMcpServerTaint(name string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PATCH", fmt.Sprintf("/api/mcp/servers/%s/taint", url.PathEscape(name)), data, nil)
}

func (r *McpResource) ListMcpTaintRules() (interface{}, error) {
	return r.client.request("GET", "/api/mcp/taint-rules", nil, nil)
}

// ── Media Resource

type MediaResource struct{ client *Client }

func (r *MediaResource) GenerateImage(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/media/image", data, nil)
}

func (r *MediaResource) GenerateMusic(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/media/music", data, nil)
}

func (r *MediaResource) ListMediaProviders() (interface{}, error) {
	return r.client.request("GET", "/api/media/providers", nil, nil)
}

func (r *MediaResource) SynthesizeSpeech(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/media/speech", data, nil)
}

// TranscribeAudio sends a raw audio/webm body. An empty contentType defaults to it.
func (r *MediaResource) TranscribeAudio(body []byte, contentType string) (interface{}, error) {
	if contentType == "" {
		contentType = "audio/webm"
	}
	return r.client.requestRaw("POST", "/api/media/transcribe", body, contentType)
}

func (r *MediaResource) SubmitVideo(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/media/video", data, nil)
}

func (r *MediaResource) PollVideoTask(task_id string, query map[string]string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/media/video/%s", url.PathEscape(task_id)), nil, query)
}

// ── Memory Resource

type MemoryResource struct{ client *Client }

func (r *MemoryResource) ExportAgentMemory(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/memory/export", url.PathEscape(id)), nil, nil)
}

func (r *MemoryResource) ImportAgentMemory(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/memory/import", url.PathEscape(id)), data, nil)
}

func (r *MemoryResource) GetAgentKv(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/memory/agents/%s/kv", url.PathEscape(id)), nil, nil)
}

func (r *MemoryResource) GetAgentKvKey(id string, key string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/memory/agents/%s/kv/%s", url.PathEscape(id), url.PathEscape(key)), nil, nil)
}

func (r *MemoryResource) SetAgentKvKey(id string, key string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/memory/agents/%s/kv/%s", url.PathEscape(id), url.PathEscape(key)), data, nil)
}

func (r *MemoryResource) DeleteAgentKvKey(id string, key string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/memory/agents/%s/kv/%s", url.PathEscape(id), url.PathEscape(key)), nil, nil)
}

func (r *MemoryResource) MemoryConfigGet() (interface{}, error) {
	return r.client.request("GET", "/api/memory/config", nil, nil)
}

func (r *MemoryResource) MemoryConfigPatch(data map[string]interface{}) (interface{}, error) {
	return r.client.request("PATCH", "/api/memory/config", data, nil)
}

// ── Models Resource

type ModelsResource struct{ client *Client }

func (r *ModelsResource) CatalogStatus() (interface{}, error) {
	return r.client.request("GET", "/api/catalog/status", nil, nil)
}

func (r *ModelsResource) CatalogUpdate() (interface{}, error) {
	return r.client.request("POST", "/api/catalog/update", nil, nil)
}

func (r *ModelsResource) ListCredentialPools() (interface{}, error) {
	return r.client.request("GET", "/api/credential-pools", nil, nil)
}

func (r *ModelsResource) ListModelRouterProfiles() (interface{}, error) {
	return r.client.request("GET", "/api/model-router/profiles", nil, nil)
}

func (r *ModelsResource) ListAllModels() (interface{}, error) {
	return r.client.request("GET", "/api/models", nil, nil)
}

func (r *ModelsResource) ListAliases() (interface{}, error) {
	return r.client.request("GET", "/api/models/aliases", nil, nil)
}

func (r *ModelsResource) CreateAlias(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/models/aliases", data, nil)
}

func (r *ModelsResource) DeleteAlias(alias string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/models/aliases/%s", url.PathEscape(alias)), nil, nil)
}

func (r *ModelsResource) AddCustomModel(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/models/custom", data, nil)
}

func (r *ModelsResource) RemoveCustomModel(id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/models/custom/%s", url.PathEscape(id)), nil, nil)
}

func (r *ModelsResource) GetModelOverrides(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/models/overrides/%s", url.PathEscape(id)), nil, nil)
}

func (r *ModelsResource) SetModelOverrides(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/models/overrides/%s", url.PathEscape(id)), data, nil)
}

func (r *ModelsResource) DeleteModelOverrides(id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/models/overrides/%s", url.PathEscape(id)), nil, nil)
}

func (r *ModelsResource) GetModel(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/models/%s", url.PathEscape(id)), nil, nil)
}

func (r *ModelsResource) ListProviders() (interface{}, error) {
	return r.client.request("GET", "/api/providers", nil, nil)
}

func (r *ModelsResource) CopilotOauthPoll(poll_id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/providers/github-copilot/oauth/poll/%s", url.PathEscape(poll_id)), nil, nil)
}

func (r *ModelsResource) CopilotOauthStart() (interface{}, error) {
	return r.client.request("POST", "/api/providers/github-copilot/oauth/start", nil, nil)
}

func (r *ModelsResource) GetProvider(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/providers/%s", url.PathEscape(name)), nil, nil)
}

func (r *ModelsResource) SetDefaultProvider(name string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/providers/%s/default", url.PathEscape(name)), data, nil)
}

func (r *ModelsResource) SetProviderDiscovery(name string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/providers/%s/discovery", url.PathEscape(name)), data, nil)
}

func (r *ModelsResource) EnableProvider(name string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/providers/%s/enable", url.PathEscape(name)), nil, nil)
}

func (r *ModelsResource) SetProviderKey(name string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/providers/%s/key", url.PathEscape(name)), data, nil)
}

func (r *ModelsResource) DeleteProviderKey(name string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/providers/%s/key", url.PathEscape(name)), nil, nil)
}

func (r *ModelsResource) TestProvider(name string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/providers/%s/test", url.PathEscape(name)), nil, nil)
}

func (r *ModelsResource) SetProviderUrl(name string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/providers/%s/url", url.PathEscape(name)), data, nil)
}

// ── Network Resource

type NetworkResource struct{ client *Client }

func (r *NetworkResource) CommsEvents(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/comms/events", nil, query)
}

func (r *NetworkResource) CommsEventsStream() <-chan map[string]interface{} {
	return r.client.stream("GET", "/api/comms/events/stream", nil, nil)
}

func (r *NetworkResource) CommsSend(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/comms/send", data, nil)
}

func (r *NetworkResource) CommsTask(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/comms/task", data, nil)
}

func (r *NetworkResource) CommsTopology() (interface{}, error) {
	return r.client.request("GET", "/api/comms/topology", nil, nil)
}

func (r *NetworkResource) NetworkStatus() (interface{}, error) {
	return r.client.request("GET", "/api/network/status", nil, nil)
}

func (r *NetworkResource) NetworkTrustedPeers() (interface{}, error) {
	return r.client.request("GET", "/api/network/trusted-peers", nil, nil)
}

func (r *NetworkResource) ListPeers(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/peers", nil, query)
}

func (r *NetworkResource) GetPeer(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/peers/%s", url.PathEscape(id)), nil, nil)
}

// ── Pairing Resource

type PairingResource struct{ client *Client }

func (r *PairingResource) PairingComplete(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/pairing/complete", data, nil)
}

func (r *PairingResource) PairingDevices() (interface{}, error) {
	return r.client.request("GET", "/api/pairing/devices", nil, nil)
}

func (r *PairingResource) PairingRemoveDevice(id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/pairing/devices/%s", url.PathEscape(id)), nil, nil)
}

func (r *PairingResource) PairingNotify(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/pairing/notify", data, nil)
}

func (r *PairingResource) PairingRequest() (interface{}, error) {
	return r.client.request("POST", "/api/pairing/request", nil, nil)
}

// ── Plugins Resource

type PluginsResource struct{ client *Client }

func (r *PluginsResource) ContextEngineChain() (interface{}, error) {
	return r.client.request("GET", "/api/context-engine/chain", nil, nil)
}

func (r *PluginsResource) ContextEngineConfig() (interface{}, error) {
	return r.client.request("GET", "/api/context-engine/config", nil, nil)
}

func (r *PluginsResource) ContextEngineHealth() (interface{}, error) {
	return r.client.request("GET", "/api/context-engine/health", nil, nil)
}

func (r *PluginsResource) ContextEngineMetrics() (interface{}, error) {
	return r.client.request("GET", "/api/context-engine/metrics", nil, nil)
}

func (r *PluginsResource) ContextEngineSandboxPolicy() (interface{}, error) {
	return r.client.request("GET", "/api/context-engine/sandbox-policy", nil, nil)
}

func (r *PluginsResource) ContextEngineTraces() (interface{}, error) {
	return r.client.request("GET", "/api/context-engine/traces", nil, nil)
}

func (r *PluginsResource) ListPlugins() (interface{}, error) {
	return r.client.request("GET", "/api/plugins", nil, nil)
}

func (r *PluginsResource) PluginDoctor() (interface{}, error) {
	return r.client.request("GET", "/api/plugins/doctor", nil, nil)
}

func (r *PluginsResource) InstallPlugin(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/plugins/install", data, nil)
}

func (r *PluginsResource) ListPluginRegistries() (interface{}, error) {
	return r.client.request("GET", "/api/plugins/registries", nil, nil)
}

func (r *PluginsResource) ScaffoldPlugin(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/plugins/scaffold", data, nil)
}

func (r *PluginsResource) UninstallPlugin(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/plugins/uninstall", data, nil)
}

func (r *PluginsResource) GetPlugin(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/plugins/%s", url.PathEscape(name)), nil, nil)
}

func (r *PluginsResource) PluginAdvancedConfig(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/plugins/%s/advanced-config", url.PathEscape(name)), nil, nil)
}

func (r *PluginsResource) DisablePlugin(name string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/plugins/%s/disable", url.PathEscape(name)), nil, nil)
}

func (r *PluginsResource) EnablePlugin(name string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/plugins/%s/enable", url.PathEscape(name)), nil, nil)
}

func (r *PluginsResource) PluginEnv(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/plugins/%s/env", url.PathEscape(name)), nil, nil)
}

func (r *PluginsResource) InstallPluginDeps(name string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/plugins/%s/install-deps", url.PathEscape(name)), nil, nil)
}

func (r *PluginsResource) LintPlugin(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/plugins/%s/lint", url.PathEscape(name)), nil, nil)
}

func (r *PluginsResource) PrewarmPlugin(name string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/plugins/%s/prewarm", url.PathEscape(name)), nil, nil)
}

func (r *PluginsResource) ReloadPlugin(name string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/plugins/%s/reload", url.PathEscape(name)), nil, nil)
}

func (r *PluginsResource) SignPlugin(name string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/plugins/%s/sign", url.PathEscape(name)), nil, nil)
}

func (r *PluginsResource) PluginStatus(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/plugins/%s/status", url.PathEscape(name)), nil, nil)
}

func (r *PluginsResource) TestPluginHook(name string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/plugins/%s/test-hook", url.PathEscape(name)), data, nil)
}

func (r *PluginsResource) UpgradePlugin(name string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/plugins/%s/upgrade", url.PathEscape(name)), data, nil)
}

// ── ProactiveMemory Resource

type ProactiveMemoryResource struct{ client *Client }

func (r *ProactiveMemoryResource) MemoryList(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/memory", nil, query)
}

func (r *ProactiveMemoryResource) MemoryAdd(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/memory", data, nil)
}

func (r *ProactiveMemoryResource) MemoryListAgent(id string, query map[string]string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/memory/agents/%s", url.PathEscape(id)), nil, query)
}

func (r *ProactiveMemoryResource) MemoryResetAgent(id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/memory/agents/%s", url.PathEscape(id)), nil, nil)
}

func (r *ProactiveMemoryResource) MemoryConsolidate(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/memory/agents/%s/consolidate", url.PathEscape(id)), nil, nil)
}

func (r *ProactiveMemoryResource) MemoryCountAgent(id string, query map[string]string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/memory/agents/%s/count", url.PathEscape(id)), nil, query)
}

func (r *ProactiveMemoryResource) MemoryDuplicates(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/memory/agents/%s/duplicates", url.PathEscape(id)), nil, nil)
}

func (r *ProactiveMemoryResource) MemoryExportAgent(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/memory/agents/%s/export", url.PathEscape(id)), nil, nil)
}

func (r *ProactiveMemoryResource) MemoryImportAgent(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/memory/agents/%s/import", url.PathEscape(id)), data, nil)
}

func (r *ProactiveMemoryResource) MemoryClearLevel(id string, level string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/memory/agents/%s/level/%s", url.PathEscape(id), url.PathEscape(level)), nil, nil)
}

func (r *ProactiveMemoryResource) MemoryQueryRelations(id string, query map[string]string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/memory/agents/%s/relations", url.PathEscape(id)), nil, query)
}

func (r *ProactiveMemoryResource) MemoryStoreRelations(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/memory/agents/%s/relations", url.PathEscape(id)), data, nil)
}

func (r *ProactiveMemoryResource) MemorySearchAgent(id string, query map[string]string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/memory/agents/%s/search", url.PathEscape(id)), nil, query)
}

func (r *ProactiveMemoryResource) MemoryStatsAgent(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/memory/agents/%s/stats", url.PathEscape(id)), nil, nil)
}

func (r *ProactiveMemoryResource) MemoryBulkDelete(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/memory/bulk-delete", data, nil)
}

func (r *ProactiveMemoryResource) MemoryCleanup() (interface{}, error) {
	return r.client.request("POST", "/api/memory/cleanup", nil, nil)
}

func (r *ProactiveMemoryResource) MemoryDecay() (interface{}, error) {
	return r.client.request("POST", "/api/memory/decay", nil, nil)
}

func (r *ProactiveMemoryResource) MemoryUpdate(memory_id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/memory/items/%s", url.PathEscape(memory_id)), data, nil)
}

func (r *ProactiveMemoryResource) MemoryDelete(memory_id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/memory/items/%s", url.PathEscape(memory_id)), nil, nil)
}

func (r *ProactiveMemoryResource) MemoryHistory(memory_id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/memory/items/%s/history", url.PathEscape(memory_id)), nil, nil)
}

func (r *ProactiveMemoryResource) MemorySearch(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/memory/search", nil, query)
}

func (r *ProactiveMemoryResource) MemoryStats() (interface{}, error) {
	return r.client.request("GET", "/api/memory/stats", nil, nil)
}

func (r *ProactiveMemoryResource) MemoryGetUser(user_id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/memory/user/%s", url.PathEscape(user_id)), nil, nil)
}

// ── Sessions Resource

type SessionsResource struct{ client *Client }

func (r *SessionsResource) FindSessionByLabel(id string, label string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/sessions/by-label/%s", url.PathEscape(id), url.PathEscape(label)), nil, nil)
}

func (r *SessionsResource) ListSessions(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/sessions", nil, query)
}

func (r *SessionsResource) SessionCleanup() (interface{}, error) {
	return r.client.request("POST", "/api/sessions/cleanup", nil, nil)
}

func (r *SessionsResource) SearchSessions(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/sessions/search", nil, query)
}

func (r *SessionsResource) GetSession(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/sessions/%s", url.PathEscape(id)), nil, nil)
}

func (r *SessionsResource) DeleteSession(id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/sessions/%s", url.PathEscape(id)), nil, nil)
}

func (r *SessionsResource) SetSessionLabel(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/sessions/%s/label", url.PathEscape(id)), data, nil)
}

func (r *SessionsResource) PatchSessionModel(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PATCH", fmt.Sprintf("/api/sessions/%s/model", url.PathEscape(id)), data, nil)
}

// ── Skills Resource

type SkillsResource struct{ client *Client }

func (r *SkillsResource) ClawhubBrowse(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/clawhub/browse", nil, query)
}

func (r *SkillsResource) ClawhubInstall(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/clawhub/install", data, nil)
}

func (r *SkillsResource) ClawhubSearch(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/clawhub/search", nil, query)
}

func (r *SkillsResource) ClawhubSkillDetail(slug string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/clawhub/skill/%s", url.PathEscape(slug)), nil, nil)
}

func (r *SkillsResource) ClawhubSkillCode(slug string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/clawhub/skill/%s/code", url.PathEscape(slug)), nil, nil)
}

func (r *SkillsResource) MarketplaceSearch(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/marketplace/search", nil, query)
}

func (r *SkillsResource) ListSkills() (interface{}, error) {
	return r.client.request("GET", "/api/skills", nil, nil)
}

func (r *SkillsResource) CreateSkill(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/skills/create", data, nil)
}

func (r *SkillsResource) InstallSkill(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/skills/install", data, nil)
}

func (r *SkillsResource) ListPendingCandidates(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/skills/pending", nil, query)
}

func (r *SkillsResource) ShowPendingCandidate(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/skills/pending/%s", url.PathEscape(id)), nil, nil)
}

func (r *SkillsResource) ApprovePendingCandidate(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/skills/pending/%s/approve", url.PathEscape(id)), nil, nil)
}

func (r *SkillsResource) ProposePendingToRegistry(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/skills/pending/%s/propose-to-registry", url.PathEscape(id)), nil, nil)
}

func (r *SkillsResource) RejectPendingCandidate(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/skills/pending/%s/reject", url.PathEscape(id)), nil, nil)
}

func (r *SkillsResource) ListSkillRegistry() (interface{}, error) {
	return r.client.request("GET", "/api/skills/registry", nil, nil)
}

func (r *SkillsResource) ReloadSkills() (interface{}, error) {
	return r.client.request("POST", "/api/skills/reload", nil, nil)
}

func (r *SkillsResource) UninstallSkill(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/skills/uninstall", data, nil)
}

func (r *SkillsResource) GetSkillDetail(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/skills/%s", url.PathEscape(name)), nil, nil)
}

func (r *SkillsResource) EvolveDeleteSkill(name string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/skills/%s/evolve/delete", url.PathEscape(name)), nil, nil)
}

func (r *SkillsResource) EvolveWriteFile(name string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/skills/%s/evolve/file", url.PathEscape(name)), data, nil)
}

func (r *SkillsResource) EvolveRemoveFile(name string, query map[string]string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/skills/%s/evolve/file", url.PathEscape(name)), nil, query)
}

func (r *SkillsResource) EvolvePatchSkill(name string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/skills/%s/evolve/patch", url.PathEscape(name)), data, nil)
}

func (r *SkillsResource) EvolveRollbackSkill(name string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/skills/%s/evolve/rollback", url.PathEscape(name)), nil, nil)
}

func (r *SkillsResource) EvolveUpdateSkill(name string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/skills/%s/evolve/update", url.PathEscape(name)), data, nil)
}

func (r *SkillsResource) GetSupportingFile(name string, query map[string]string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/skills/%s/file", url.PathEscape(name)), nil, query)
}

func (r *SkillsResource) ProposeSkillToRegistry(name string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/skills/%s/propose", url.PathEscape(name)), nil, nil)
}

func (r *SkillsResource) ListTools() (interface{}, error) {
	return r.client.request("GET", "/api/tools", nil, nil)
}

func (r *SkillsResource) GetTool(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/tools/%s", url.PathEscape(name)), nil, nil)
}

// ── System Resource

type SystemResource struct{ client *Client }

func (r *SystemResource) AuditExport(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/audit/export", nil, query)
}

func (r *SystemResource) AuditQuery(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/audit/query", nil, query)
}

func (r *SystemResource) AuditRecent() (interface{}, error) {
	return r.client.request("GET", "/api/audit/recent", nil, nil)
}

func (r *SystemResource) AuditVerify() (interface{}, error) {
	return r.client.request("GET", "/api/audit/verify", nil, nil)
}

func (r *SystemResource) Check(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/authz/check", nil, query)
}

func (r *SystemResource) EffectivePermissions(user_id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/authz/effective/%s", url.PathEscape(user_id)), nil, nil)
}

func (r *SystemResource) Whoami() (interface{}, error) {
	return r.client.request("GET", "/api/authz/whoami", nil, nil)
}

func (r *SystemResource) CreateBackup() (interface{}, error) {
	return r.client.request("POST", "/api/backup", nil, nil)
}

func (r *SystemResource) ListBackups() (interface{}, error) {
	return r.client.request("GET", "/api/backups", nil, nil)
}

func (r *SystemResource) DeleteBackup(filename string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/backups/%s", url.PathEscape(filename)), nil, nil)
}

func (r *SystemResource) ListBindings() (interface{}, error) {
	return r.client.request("GET", "/api/bindings", nil, nil)
}

func (r *SystemResource) AddBinding(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/bindings", data, nil)
}

func (r *SystemResource) RemoveBinding(index string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/bindings/%s", url.PathEscape(index)), nil, nil)
}

func (r *SystemResource) ListCommands() (interface{}, error) {
	return r.client.request("GET", "/api/commands", nil, nil)
}

func (r *SystemResource) GetCommand(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/commands/%s", url.PathEscape(name)), nil, nil)
}

func (r *SystemResource) GetConfig() (interface{}, error) {
	return r.client.request("GET", "/api/config", nil, nil)
}

func (r *SystemResource) ExportConfig() (interface{}, error) {
	return r.client.request("GET", "/api/config/export", nil, nil)
}

func (r *SystemResource) ConfigReload() (interface{}, error) {
	return r.client.request("POST", "/api/config/reload", nil, nil)
}

func (r *SystemResource) ConfigSchema() (interface{}, error) {
	return r.client.request("GET", "/api/config/schema", nil, nil)
}

func (r *SystemResource) ConfigSet(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/config/set", data, nil)
}

func (r *SystemResource) ConfigStatus() (interface{}, error) {
	return r.client.request("GET", "/api/config/status", nil, nil)
}

func (r *SystemResource) Health() (interface{}, error) {
	return r.client.request("GET", "/api/health", nil, nil)
}

func (r *SystemResource) HealthDetail() (interface{}, error) {
	return r.client.request("GET", "/api/health/detail", nil, nil)
}

func (r *SystemResource) QuickInit() (interface{}, error) {
	return r.client.request("POST", "/api/init", nil, nil)
}

func (r *SystemResource) LogsStream() <-chan map[string]interface{} {
	return r.client.stream("GET", "/api/logs/stream", nil, nil)
}

func (r *SystemResource) PrometheusMetrics() (interface{}, error) {
	return r.client.request("GET", "/api/metrics", nil, nil)
}

func (r *SystemResource) RunMigrate(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/migrate", data, nil)
}

func (r *SystemResource) MigrateDetect() (interface{}, error) {
	return r.client.request("GET", "/api/migrate/detect", nil, nil)
}

func (r *SystemResource) MigrateScan(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/migrate/scan", data, nil)
}

func (r *SystemResource) ListProfiles() (interface{}, error) {
	return r.client.request("GET", "/api/profiles", nil, nil)
}

func (r *SystemResource) GetProfile(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/profiles/%s", url.PathEscape(name)), nil, nil)
}

func (r *SystemResource) ProvisioningStatus() (interface{}, error) {
	return r.client.request("GET", "/api/provisioning/status", nil, nil)
}

func (r *SystemResource) QueueStatus() (interface{}, error) {
	return r.client.request("GET", "/api/queue/status", nil, nil)
}

func (r *SystemResource) Ready() (interface{}, error) {
	return r.client.request("GET", "/api/ready", nil, nil)
}

func (r *SystemResource) RestoreBackup(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/restore", data, nil)
}

func (r *SystemResource) SecurityStatus() (interface{}, error) {
	return r.client.request("GET", "/api/security", nil, nil)
}

func (r *SystemResource) Shutdown() (interface{}, error) {
	return r.client.request("POST", "/api/shutdown", nil, nil)
}

func (r *SystemResource) Status() (interface{}, error) {
	return r.client.request("GET", "/api/status", nil, nil)
}

func (r *SystemResource) ListAgentTemplates() (interface{}, error) {
	return r.client.request("GET", "/api/templates", nil, nil)
}

func (r *SystemResource) CreateAgentType(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/templates", data, nil)
}

func (r *SystemResource) GetAgentTemplate(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/templates/%s", url.PathEscape(name)), nil, nil)
}

func (r *SystemResource) UpdateAgentType(name string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/templates/%s", url.PathEscape(name)), data, nil)
}

func (r *SystemResource) DeleteAgentType(name string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/templates/%s", url.PathEscape(name)), nil, nil)
}

func (r *SystemResource) ListTemplateHistory(name string, query map[string]string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/templates/%s/history", url.PathEscape(name)), nil, query)
}

func (r *SystemResource) RestoreTemplateVersion(name string, version_id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/templates/%s/history/%s/restore", url.PathEscape(name), url.PathEscape(version_id)), nil, nil)
}

func (r *SystemResource) PromoteAgentType(name string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/templates/%s/promote", url.PathEscape(name)), nil, nil)
}

func (r *SystemResource) GetAgentTypeRegistryDiff(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/templates/%s/registry-diff", url.PathEscape(name)), nil, nil)
}

func (r *SystemResource) RestoreAgentTypeFromRegistry(name string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/templates/%s/restore", url.PathEscape(name)), nil, nil)
}

func (r *SystemResource) GetAgentTemplateToml(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/templates/%s/toml", url.PathEscape(name)), nil, nil)
}

// PutAgentTemplateToml sends a raw text/plain body. An empty contentType defaults to it.
func (r *SystemResource) PutAgentTemplateToml(name string, body []byte, contentType string) (interface{}, error) {
	if contentType == "" {
		contentType = "text/plain"
	}
	return r.client.requestRaw("PUT", fmt.Sprintf("/api/templates/%s/toml", url.PathEscape(name)), body, contentType)
}

// PostAgentTemplateToml sends a raw text/plain body. An empty contentType defaults to it.
func (r *SystemResource) PostAgentTemplateToml(name string, body []byte, contentType string) (interface{}, error) {
	if contentType == "" {
		contentType = "text/plain"
	}
	return r.client.requestRaw("POST", fmt.Sprintf("/api/templates/%s/toml", url.PathEscape(name)), body, contentType)
}

func (r *SystemResource) Version() (interface{}, error) {
	return r.client.request("GET", "/api/version", nil, nil)
}

func (r *SystemResource) ApiVersions() (interface{}, error) {
	return r.client.request("GET", "/api/versions", nil, nil)
}

// ── Tasks Resource

type TasksResource struct{ client *Client }

func (r *TasksResource) TaskQueuePostRoot(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/tasks", data, nil)
}

// ── Tools Resource

type ToolsResource struct{ client *Client }

func (r *ToolsResource) InvokeTool(name string, data map[string]interface{}, query map[string]string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/tools/%s/invoke", url.PathEscape(name)), data, query)
}

// ── Users Resource

type UsersResource struct{ client *Client }

func (r *UsersResource) ListUsers() (interface{}, error) {
	return r.client.request("GET", "/api/users", nil, nil)
}

func (r *UsersResource) CreateUser(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/users", data, nil)
}

func (r *UsersResource) ImportUsers(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/users/import", data, nil)
}

func (r *UsersResource) ServeMyAvatar() (interface{}, error) {
	return r.client.request("GET", "/api/users/me/avatar", nil, nil)
}

func (r *UsersResource) GetUser(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/users/%s", url.PathEscape(name)), nil, nil)
}

func (r *UsersResource) UpdateUser(name string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/users/%s", url.PathEscape(name)), data, nil)
}

func (r *UsersResource) DeleteUser(name string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/users/%s", url.PathEscape(name)), nil, nil)
}

func (r *UsersResource) ServeUserAvatar(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/users/%s/avatar", url.PathEscape(name)), nil, nil)
}

// UploadUserAvatar sends a raw application/octet-stream body. An empty contentType defaults to it.
func (r *UsersResource) UploadUserAvatar(name string, body []byte, contentType string) (interface{}, error) {
	if contentType == "" {
		contentType = "application/octet-stream"
	}
	return r.client.requestRaw("POST", fmt.Sprintf("/api/users/%s/avatar", url.PathEscape(name)), body, contentType)
}

func (r *UsersResource) DeleteUserAvatar(name string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/users/%s/avatar", url.PathEscape(name)), nil, nil)
}

func (r *UsersResource) UpdateUserIdentity(name string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PATCH", fmt.Sprintf("/api/users/%s/identity", url.PathEscape(name)), data, nil)
}

func (r *UsersResource) GetUserPolicy(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/users/%s/policy", url.PathEscape(name)), nil, nil)
}

func (r *UsersResource) UpdateUserPolicy(name string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/users/%s/policy", url.PathEscape(name)), data, nil)
}

func (r *UsersResource) ListUserProviderKeys(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/users/%s/provider-keys", url.PathEscape(name)), nil, nil)
}

func (r *UsersResource) SetUserProviderKey(name string, provider string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/users/%s/provider-keys/%s", url.PathEscape(name), url.PathEscape(provider)), data, nil)
}

func (r *UsersResource) DeleteUserProviderKey(name string, provider string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/users/%s/provider-keys/%s", url.PathEscape(name), url.PathEscape(provider)), nil, nil)
}

func (r *UsersResource) RotateUserKey(name string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/users/%s/rotate-key", url.PathEscape(name)), nil, nil)
}

// ── Vault Resource

type VaultResource struct{ client *Client }

func (r *VaultResource) VaultListKeys() (interface{}, error) {
	return r.client.request("GET", "/api/vault/keys", nil, nil)
}

func (r *VaultResource) VaultPutKey(key string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/vault/keys/%s", url.PathEscape(key)), data, nil)
}

func (r *VaultResource) VaultDeleteKey(key string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/vault/keys/%s", url.PathEscape(key)), nil, nil)
}

// ── Webhooks Resource

type WebhooksResource struct{ client *Client }

func (r *WebhooksResource) WebhookAgent(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/hooks/agent", data, nil)
}

func (r *WebhooksResource) WebhookWake(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/hooks/wake", data, nil)
}

// ── Workflows Resource

type WorkflowsResource struct{ client *Client }

func (r *WorkflowsResource) ListCronJobs() (interface{}, error) {
	return r.client.request("GET", "/api/cron/jobs", nil, nil)
}

func (r *WorkflowsResource) CreateCronJob(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/cron/jobs", data, nil)
}

func (r *WorkflowsResource) GetCronJob(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/cron/jobs/%s", url.PathEscape(id)), nil, nil)
}

func (r *WorkflowsResource) UpdateCronJob(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/cron/jobs/%s", url.PathEscape(id)), data, nil)
}

func (r *WorkflowsResource) DeleteCronJob(id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/cron/jobs/%s", url.PathEscape(id)), nil, nil)
}

func (r *WorkflowsResource) ToggleCronJob(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/cron/jobs/%s/enable", url.PathEscape(id)), data, nil)
}

func (r *WorkflowsResource) CronJobStatus(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/cron/jobs/%s/status", url.PathEscape(id)), nil, nil)
}

func (r *WorkflowsResource) ListSchedules() (interface{}, error) {
	return r.client.request("GET", "/api/schedules", nil, nil)
}

func (r *WorkflowsResource) CreateSchedule(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/schedules", data, nil)
}

func (r *WorkflowsResource) GetSchedule(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/schedules/%s", url.PathEscape(id)), nil, nil)
}

func (r *WorkflowsResource) UpdateSchedule(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/schedules/%s", url.PathEscape(id)), data, nil)
}

func (r *WorkflowsResource) DeleteSchedule(id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/schedules/%s", url.PathEscape(id)), nil, nil)
}

func (r *WorkflowsResource) RunSchedule(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/schedules/%s/run", url.PathEscape(id)), nil, nil)
}

func (r *WorkflowsResource) ListTriggers(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/triggers", nil, query)
}

func (r *WorkflowsResource) CreateTrigger(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/triggers", data, nil)
}

func (r *WorkflowsResource) GetTrigger(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/triggers/%s", url.PathEscape(id)), nil, nil)
}

func (r *WorkflowsResource) DeleteTrigger(id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/triggers/%s", url.PathEscape(id)), nil, nil)
}

func (r *WorkflowsResource) UpdateTrigger(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PATCH", fmt.Sprintf("/api/triggers/%s", url.PathEscape(id)), data, nil)
}

func (r *WorkflowsResource) ListWorkflowTemplates(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/workflow-templates", nil, query)
}

func (r *WorkflowsResource) GetWorkflowTemplate(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/workflow-templates/%s", url.PathEscape(id)), nil, nil)
}

func (r *WorkflowsResource) InstantiateTemplate(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/workflow-templates/%s/instantiate", url.PathEscape(id)), data, nil)
}

func (r *WorkflowsResource) ListWorkflows() (interface{}, error) {
	return r.client.request("GET", "/api/workflows", nil, nil)
}

func (r *WorkflowsResource) CreateWorkflow(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/workflows", data, nil)
}

func (r *WorkflowsResource) GetWorkflowRun(run_id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/workflows/runs/%s", url.PathEscape(run_id)), nil, nil)
}

func (r *WorkflowsResource) CancelWorkflowRun(run_id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/workflows/runs/%s/cancel", url.PathEscape(run_id)), nil, nil)
}

func (r *WorkflowsResource) OperatorActionWorkflowRun(run_id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/workflows/runs/%s/operator", url.PathEscape(run_id)), data, nil)
}

func (r *WorkflowsResource) PauseWorkflowRun(run_id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/workflows/runs/%s/pause", url.PathEscape(run_id)), data, nil)
}

func (r *WorkflowsResource) RerunWorkflowRun(run_id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/workflows/runs/%s/rerun", url.PathEscape(run_id)), nil, nil)
}

func (r *WorkflowsResource) ResumeWorkflowRun(run_id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/workflows/runs/%s/resume", url.PathEscape(run_id)), data, nil)
}

func (r *WorkflowsResource) GetWorkflow(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/workflows/%s", url.PathEscape(id)), nil, nil)
}

func (r *WorkflowsResource) UpdateWorkflow(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/workflows/%s", url.PathEscape(id)), data, nil)
}

func (r *WorkflowsResource) DeleteWorkflow(id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/workflows/%s", url.PathEscape(id)), nil, nil)
}

func (r *WorkflowsResource) DryRunWorkflow(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/workflows/%s/dry-run", url.PathEscape(id)), data, nil)
}

func (r *WorkflowsResource) RunWorkflow(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/workflows/%s/run", url.PathEscape(id)), data, nil)
}

func (r *WorkflowsResource) ListWorkflowRuns(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/workflows/%s/runs", url.PathEscape(id)), nil, nil)
}

func (r *WorkflowsResource) SaveWorkflowAsTemplate(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/workflows/%s/save-as-template", url.PathEscape(id)), nil, nil)
}

