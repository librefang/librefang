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
	Hands *HandsResource
	Mcp *McpResource
	Memory *MemoryResource
	Models *ModelsResource
	Network *NetworkResource
	Pairing *PairingResource
	ProactiveMemory *ProactiveMemoryResource
	Sessions *SessionsResource
	Skills *SkillsResource
	System *SystemResource
	Tools *ToolsResource
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
		c.Hands = &HandsResource{client: c}
		c.Mcp = &McpResource{client: c}
		c.Memory = &MemoryResource{client: c}
		c.Models = &ModelsResource{client: c}
		c.Network = &NetworkResource{client: c}
		c.Pairing = &PairingResource{client: c}
		c.ProactiveMemory = &ProactiveMemoryResource{client: c}
		c.Sessions = &SessionsResource{client: c}
		c.Skills = &SkillsResource{client: c}
		c.System = &SystemResource{client: c}
		c.Tools = &ToolsResource{client: c}
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
	urlStr := c.BaseURL + c.withQuery(path, query)
	var bodyBytes []byte
	if body != nil {
		b, err := json.Marshal(body)
		if err != nil {
			return nil, fmt.Errorf("marshal: %w", err)
		}
		bodyBytes = b
	}
	req, err := http.NewRequest(method, urlStr, bytes.NewReader(bodyBytes))
	if err != nil {
		return nil, err
	}
	for k, v := range c.Headers {
		req.Header.Set(k, v)
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
			b, _ := json.Marshal(body)
			bodyBytes = b
		}
		req, _ := http.NewRequest(method, urlStr, bytes.NewReader(bodyBytes))
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
	return r.client.request("GET", fmt.Sprintf("/api/a2a/agents/%s", id), nil, nil)
}

func (r *A2AResource) A2ADiscoverExternal(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/a2a/discover", data, nil)
}

func (r *A2AResource) A2ASendExternal(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/a2a/send", data, nil)
}

func (r *A2AResource) A2AExternalTaskStatus(id string, query map[string]string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/a2a/tasks/%s/status", id), nil, query)
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

func (r *AgentsResource) GetAgent(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s", id), nil, nil)
}

func (r *AgentsResource) KillAgent(id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/agents/%s", id), nil, nil)
}

func (r *AgentsResource) PatchAgent(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PATCH", fmt.Sprintf("/api/agents/%s", id), data, nil)
}

func (r *AgentsResource) CloneAgent(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/clone", id), data, nil)
}

func (r *AgentsResource) PatchAgentConfig(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PATCH", fmt.Sprintf("/api/agents/%s/config", id), data, nil)
}

func (r *AgentsResource) GetAgentDeliveries(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/deliveries", id), nil, nil)
}

func (r *AgentsResource) ListAgentFiles(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/files", id), nil, nil)
}

func (r *AgentsResource) GetAgentFile(id string, filename string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/files/%s", id, filename), nil, nil)
}

func (r *AgentsResource) SetAgentFile(id string, filename string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/agents/%s/files/%s", id, filename), data, nil)
}

func (r *AgentsResource) DeleteAgentFile(id string, filename string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/agents/%s/files/%s", id, filename), nil, nil)
}

func (r *AgentsResource) ClearAgentHistory(id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/agents/%s/history", id), nil, nil)
}

func (r *AgentsResource) UpdateAgentIdentity(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PATCH", fmt.Sprintf("/api/agents/%s/identity", id), data, nil)
}

func (r *AgentsResource) GetAgentMcpServers(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/mcp_servers", id), nil, nil)
}

func (r *AgentsResource) SetAgentMcpServers(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/agents/%s/mcp_servers", id), data, nil)
}

func (r *AgentsResource) SendMessage(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/message", id), data, nil)
}

func (r *AgentsResource) SendMessageStream(id string, data map[string]interface{}) <-chan map[string]interface{} {
	return r.client.stream("POST", fmt.Sprintf("/api/agents/%s/message/stream", id), data, nil)
}

func (r *AgentsResource) SetAgentMode(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/agents/%s/mode", id), data, nil)
}

func (r *AgentsResource) SetModel(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/agents/%s/model", id), data, nil)
}

func (r *AgentsResource) ListAgentRuntime(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/runtime", id), nil, nil)
}

func (r *AgentsResource) GetAgentSession(id string, query map[string]string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/session", id), nil, query)
}

func (r *AgentsResource) CompactSession(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/session/compact", id), nil, nil)
}

func (r *AgentsResource) RebootSession(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/session/reboot", id), nil, nil)
}

func (r *AgentsResource) ResetSession(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/session/reset", id), nil, nil)
}

func (r *AgentsResource) ListAgentSessions(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/sessions", id), nil, nil)
}

func (r *AgentsResource) CreateAgentSession(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/sessions", id), data, nil)
}

func (r *AgentsResource) ImportSession(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/sessions/import", id), data, nil)
}

func (r *AgentsResource) ExportSession(id string, session_id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/sessions/%s/export", id, session_id), nil, nil)
}

func (r *AgentsResource) StopSession(id string, session_id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/sessions/%s/stop", id, session_id), nil, nil)
}

func (r *AgentsResource) AttachSessionStream(id string, session_id string) <-chan map[string]interface{} {
	return r.client.stream("GET", fmt.Sprintf("/api/agents/%s/sessions/%s/stream", id, session_id), nil, nil)
}

func (r *AgentsResource) SwitchAgentSession(id string, session_id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/sessions/%s/switch", id, session_id), nil, nil)
}

func (r *AgentsResource) ExportSessionTrajectory(id string, session_id string, query map[string]string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/sessions/%s/trajectory", id, session_id), nil, query)
}

func (r *AgentsResource) GetAgentSkills(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/skills", id), nil, nil)
}

func (r *AgentsResource) SetAgentSkills(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/agents/%s/skills", id), data, nil)
}

func (r *AgentsResource) StopAgent(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/stop", id), nil, nil)
}

func (r *AgentsResource) GetAgentTools(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/tools", id), nil, nil)
}

func (r *AgentsResource) SetAgentTools(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/agents/%s/tools", id), data, nil)
}

func (r *AgentsResource) GetAgentTraces(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/traces", id), nil, nil)
}

func (r *AgentsResource) UpdateAgent(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/agents/%s/update", id), data, nil)
}

func (r *AgentsResource) UploadFile(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/upload", id), data, nil)
}

func (r *AgentsResource) ServeUpload(file_id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/uploads/%s", file_id), nil, nil)
}

// ── Approvals Resource

type ApprovalsResource struct{ client *Client }

func (r *ApprovalsResource) ListApprovals() (interface{}, error) {
	return r.client.request("GET", "/api/approvals", nil, nil)
}

func (r *ApprovalsResource) CreateApproval(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/approvals", data, nil)
}

func (r *ApprovalsResource) GetApproval(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/approvals/%s", id), nil, nil)
}

func (r *ApprovalsResource) ApproveRequest(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/approvals/%s/approve", id), data, nil)
}

func (r *ApprovalsResource) RejectRequest(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/approvals/%s/reject", id), nil, nil)
}

// ── Auth Resource

type AuthResource struct{ client *Client }

func (r *AuthResource) AuthCallback() (interface{}, error) {
	return r.client.request("GET", "/api/auth/callback", nil, nil)
}

func (r *AuthResource) AuthCallbackPost(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/auth/callback", data, nil)
}

func (r *AuthResource) AuthIntrospect(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/auth/introspect", data, nil)
}

func (r *AuthResource) AuthLogin() (interface{}, error) {
	return r.client.request("GET", "/api/auth/login", nil, nil)
}

func (r *AuthResource) AuthLoginProvider(provider string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/auth/login/%s", provider), nil, nil)
}

func (r *AuthResource) AuthProviders() (interface{}, error) {
	return r.client.request("GET", "/api/auth/providers", nil, nil)
}

func (r *AuthResource) AuthUserinfo() (interface{}, error) {
	return r.client.request("GET", "/api/auth/userinfo", nil, nil)
}

// ── AutoDream Resource

type AutoDreamResource struct{ client *Client }

func (r *AutoDreamResource) AutoDreamAbort(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/auto-dream/agents/%s/abort", id), nil, nil)
}

func (r *AutoDreamResource) AutoDreamSetEnabled(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/auto-dream/agents/%s/enabled", id), data, nil)
}

func (r *AutoDreamResource) AutoDreamTrigger(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/auto-dream/agents/%s/trigger", id), nil, nil)
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
	return r.client.request("GET", fmt.Sprintf("/api/budget/agents/%s", id), nil, nil)
}

func (r *BudgetResource) UpdateAgentBudget(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/budget/agents/%s", id), data, nil)
}

func (r *BudgetResource) UsageStats() (interface{}, error) {
	return r.client.request("GET", "/api/usage", nil, nil)
}

func (r *BudgetResource) UsageByModel() (interface{}, error) {
	return r.client.request("GET", "/api/usage/by-model", nil, nil)
}

func (r *BudgetResource) UsageDaily() (interface{}, error) {
	return r.client.request("GET", "/api/usage/daily", nil, nil)
}

func (r *BudgetResource) UsageSummary() (interface{}, error) {
	return r.client.request("GET", "/api/usage/summary", nil, nil)
}

// ── Channels Resource

type ChannelsResource struct{ client *Client }

func (r *ChannelsResource) ListChannels() (interface{}, error) {
	return r.client.request("GET", "/api/channels", nil, nil)
}

func (r *ChannelsResource) ReloadChannels() (interface{}, error) {
	return r.client.request("POST", "/api/channels/reload", nil, nil)
}

func (r *ChannelsResource) WechatQrStart() (interface{}, error) {
	return r.client.request("POST", "/api/channels/wechat/qr/start", nil, nil)
}

func (r *ChannelsResource) WechatQrStatus(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/channels/wechat/qr/status", nil, query)
}

func (r *ChannelsResource) WhatsappQrStart() (interface{}, error) {
	return r.client.request("POST", "/api/channels/whatsapp/qr/start", nil, nil)
}

func (r *ChannelsResource) WhatsappQrStatus(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/channels/whatsapp/qr/status", nil, query)
}

func (r *ChannelsResource) ConfigureChannel(name string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/channels/%s/configure", name), data, nil)
}

func (r *ChannelsResource) RemoveChannel(name string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/channels/%s/configure", name), nil, nil)
}

func (r *ChannelsResource) TestChannel(name string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/channels/%s/test", name), data, nil)
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
	return r.client.request("GET", fmt.Sprintf("/api/extensions/%s", name), nil, nil)
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
	return r.client.request("DELETE", fmt.Sprintf("/api/hands/instances/%s", id), nil, nil)
}

func (r *HandsResource) HandInstanceBrowser(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/hands/instances/%s/browser", id), nil, nil)
}

func (r *HandsResource) PauseHand(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/hands/instances/%s/pause", id), nil, nil)
}

func (r *HandsResource) ResumeHand(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/hands/instances/%s/resume", id), nil, nil)
}

func (r *HandsResource) HandStats(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/hands/instances/%s/stats", id), nil, nil)
}

func (r *HandsResource) ReloadHands() (interface{}, error) {
	return r.client.request("POST", "/api/hands/reload", nil, nil)
}

func (r *HandsResource) GetHand(hand_id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/hands/%s", hand_id), nil, nil)
}

func (r *HandsResource) ActivateHand(hand_id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/hands/%s/activate", hand_id), data, nil)
}

func (r *HandsResource) CheckHandDeps(hand_id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/hands/%s/check-deps", hand_id), nil, nil)
}

func (r *HandsResource) InstallHandDeps(hand_id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/hands/%s/install-deps", hand_id), nil, nil)
}

func (r *HandsResource) GetHandSettings(hand_id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/hands/%s/settings", hand_id), nil, nil)
}

func (r *HandsResource) UpdateHandSettings(hand_id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/hands/%s/settings", hand_id), data, nil)
}

// ── Mcp Resource

type McpResource struct{ client *Client }

func (r *McpResource) ListMcpCatalog() (interface{}, error) {
	return r.client.request("GET", "/api/mcp/catalog", nil, nil)
}

func (r *McpResource) GetMcpCatalogEntry(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/mcp/catalog/%s", id), nil, nil)
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
	return r.client.request("GET", fmt.Sprintf("/api/mcp/servers/%s", name), nil, nil)
}

func (r *McpResource) UpdateMcpServer(name string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/mcp/servers/%s", name), data, nil)
}

func (r *McpResource) DeleteMcpServer(name string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/mcp/servers/%s", name), nil, nil)
}

func (r *McpResource) ReconnectMcpServerHandler(name string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/mcp/servers/%s/reconnect", name), nil, nil)
}

func (r *McpResource) ListMcpTaintRules() (interface{}, error) {
	return r.client.request("GET", "/api/mcp/taint-rules", nil, nil)
}

// ── Memory Resource

type MemoryResource struct{ client *Client }

func (r *MemoryResource) ExportAgentMemory(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/memory/export", id), nil, nil)
}

func (r *MemoryResource) ImportAgentMemory(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/agents/%s/memory/import", id), data, nil)
}

func (r *MemoryResource) GetAgentKv(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/memory/agents/%s/kv", id), nil, nil)
}

func (r *MemoryResource) GetAgentKvKey(id string, key string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/memory/agents/%s/kv/%s", id, key), nil, nil)
}

func (r *MemoryResource) SetAgentKvKey(id string, key string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/memory/agents/%s/kv/%s", id, key), data, nil)
}

func (r *MemoryResource) DeleteAgentKvKey(id string, key string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/memory/agents/%s/kv/%s", id, key), nil, nil)
}

// ── Models Resource

type ModelsResource struct{ client *Client }

func (r *ModelsResource) CatalogStatus() (interface{}, error) {
	return r.client.request("GET", "/api/catalog/status", nil, nil)
}

func (r *ModelsResource) CatalogUpdate() (interface{}, error) {
	return r.client.request("POST", "/api/catalog/update", nil, nil)
}

func (r *ModelsResource) ListModels() (interface{}, error) {
	return r.client.request("GET", "/api/models", nil, nil)
}

func (r *ModelsResource) ListAliases() (interface{}, error) {
	return r.client.request("GET", "/api/models/aliases", nil, nil)
}

func (r *ModelsResource) CreateAlias(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/models/aliases", data, nil)
}

func (r *ModelsResource) DeleteAlias(alias string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/models/aliases/%s", alias), nil, nil)
}

func (r *ModelsResource) AddCustomModel(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/models/custom", data, nil)
}

func (r *ModelsResource) RemoveCustomModel(id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/models/custom/%s", id), nil, nil)
}

func (r *ModelsResource) GetModel(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/models/%s", id), nil, nil)
}

func (r *ModelsResource) ListProviders() (interface{}, error) {
	return r.client.request("GET", "/api/providers", nil, nil)
}

func (r *ModelsResource) CopilotOauthPoll(poll_id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/providers/github-copilot/oauth/poll/%s", poll_id), nil, nil)
}

func (r *ModelsResource) CopilotOauthStart() (interface{}, error) {
	return r.client.request("POST", "/api/providers/github-copilot/oauth/start", nil, nil)
}

func (r *ModelsResource) GetProvider(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/providers/%s", name), nil, nil)
}

func (r *ModelsResource) SetDefaultProvider(name string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/providers/%s/default", name), data, nil)
}

func (r *ModelsResource) SetProviderKey(name string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/providers/%s/key", name), data, nil)
}

func (r *ModelsResource) DeleteProviderKey(name string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/providers/%s/key", name), nil, nil)
}

func (r *ModelsResource) TestProvider(name string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/providers/%s/test", name), nil, nil)
}

func (r *ModelsResource) SetProviderUrl(name string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/providers/%s/url", name), data, nil)
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

func (r *NetworkResource) ListPeers() (interface{}, error) {
	return r.client.request("GET", "/api/peers", nil, nil)
}

func (r *NetworkResource) GetPeer(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/peers/%s", id), nil, nil)
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
	return r.client.request("DELETE", fmt.Sprintf("/api/pairing/devices/%s", id), nil, nil)
}

func (r *PairingResource) PairingNotify(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/pairing/notify", data, nil)
}

func (r *PairingResource) PairingRequest() (interface{}, error) {
	return r.client.request("POST", "/api/pairing/request", nil, nil)
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
	return r.client.request("GET", fmt.Sprintf("/api/memory/agents/%s", id), nil, query)
}

func (r *ProactiveMemoryResource) MemoryResetAgent(id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/memory/agents/%s", id), nil, nil)
}

func (r *ProactiveMemoryResource) MemoryConsolidate(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/memory/agents/%s/consolidate", id), nil, nil)
}

func (r *ProactiveMemoryResource) MemoryDuplicates(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/memory/agents/%s/duplicates", id), nil, nil)
}

func (r *ProactiveMemoryResource) MemoryExportAgent(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/memory/agents/%s/export", id), nil, nil)
}

func (r *ProactiveMemoryResource) MemoryImportAgent(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/memory/agents/%s/import", id), data, nil)
}

func (r *ProactiveMemoryResource) MemoryClearLevel(id string, level string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/memory/agents/%s/level/%s", id, level), nil, nil)
}

func (r *ProactiveMemoryResource) MemorySearchAgent(id string, query map[string]string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/memory/agents/%s/search", id), nil, query)
}

func (r *ProactiveMemoryResource) MemoryStatsAgent(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/memory/agents/%s/stats", id), nil, nil)
}

func (r *ProactiveMemoryResource) MemoryCleanup() (interface{}, error) {
	return r.client.request("POST", "/api/memory/cleanup", nil, nil)
}

func (r *ProactiveMemoryResource) MemoryUpdate(memory_id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/memory/items/%s", memory_id), data, nil)
}

func (r *ProactiveMemoryResource) MemoryDelete(memory_id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/memory/items/%s", memory_id), nil, nil)
}

func (r *ProactiveMemoryResource) MemoryHistory(memory_id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/memory/items/%s/history", memory_id), nil, nil)
}

func (r *ProactiveMemoryResource) MemorySearch(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/memory/search", nil, query)
}

func (r *ProactiveMemoryResource) MemoryStats() (interface{}, error) {
	return r.client.request("GET", "/api/memory/stats", nil, nil)
}

func (r *ProactiveMemoryResource) MemoryGetUser(user_id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/memory/user/%s", user_id), nil, nil)
}

// ── Sessions Resource

type SessionsResource struct{ client *Client }

func (r *SessionsResource) FindSessionByLabel(id string, label string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/agents/%s/sessions/by-label/%s", id, label), nil, nil)
}

func (r *SessionsResource) ListSessions() (interface{}, error) {
	return r.client.request("GET", "/api/sessions", nil, nil)
}

func (r *SessionsResource) SessionCleanup() (interface{}, error) {
	return r.client.request("POST", "/api/sessions/cleanup", nil, nil)
}

func (r *SessionsResource) GetSession(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/sessions/%s", id), nil, nil)
}

func (r *SessionsResource) DeleteSession(id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/sessions/%s", id), nil, nil)
}

func (r *SessionsResource) SetSessionLabel(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/sessions/%s/label", id), data, nil)
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
	return r.client.request("GET", fmt.Sprintf("/api/clawhub/skill/%s", slug), nil, nil)
}

func (r *SkillsResource) ClawhubSkillCode(slug string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/clawhub/skill/%s/code", slug), nil, nil)
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

func (r *SkillsResource) UninstallSkill(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/skills/uninstall", data, nil)
}

func (r *SkillsResource) ListTools() (interface{}, error) {
	return r.client.request("GET", "/api/tools", nil, nil)
}

func (r *SkillsResource) GetTool(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/tools/%s", name), nil, nil)
}

// ── System Resource

type SystemResource struct{ client *Client }

func (r *SystemResource) AuditRecent() (interface{}, error) {
	return r.client.request("GET", "/api/audit/recent", nil, nil)
}

func (r *SystemResource) AuditVerify() (interface{}, error) {
	return r.client.request("GET", "/api/audit/verify", nil, nil)
}

func (r *SystemResource) CreateBackup() (interface{}, error) {
	return r.client.request("POST", "/api/backup", nil, nil)
}

func (r *SystemResource) ListBackups() (interface{}, error) {
	return r.client.request("GET", "/api/backups", nil, nil)
}

func (r *SystemResource) DeleteBackup(filename string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/backups/%s", filename), nil, nil)
}

func (r *SystemResource) ListBindings() (interface{}, error) {
	return r.client.request("GET", "/api/bindings", nil, nil)
}

func (r *SystemResource) AddBinding(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/bindings", data, nil)
}

func (r *SystemResource) RemoveBinding(index string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/bindings/%s", index), nil, nil)
}

func (r *SystemResource) ListCommands() (interface{}, error) {
	return r.client.request("GET", "/api/commands", nil, nil)
}

func (r *SystemResource) GetCommand(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/commands/%s", name), nil, nil)
}

func (r *SystemResource) GetConfig() (interface{}, error) {
	return r.client.request("GET", "/api/config", nil, nil)
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
	return r.client.request("GET", fmt.Sprintf("/api/profiles/%s", name), nil, nil)
}

func (r *SystemResource) QueueStatus() (interface{}, error) {
	return r.client.request("GET", "/api/queue/status", nil, nil)
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

func (r *SystemResource) GetAgentTemplate(name string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/templates/%s", name), nil, nil)
}

func (r *SystemResource) Version() (interface{}, error) {
	return r.client.request("GET", "/api/version", nil, nil)
}

func (r *SystemResource) ApiVersions() (interface{}, error) {
	return r.client.request("GET", "/api/versions", nil, nil)
}

// ── Tools Resource

type ToolsResource struct{ client *Client }

func (r *ToolsResource) InvokeTool(name string, data map[string]interface{}, query map[string]string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/tools/%s/invoke", name), data, query)
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

func (r *WorkflowsResource) UpdateCronJob(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/cron/jobs/%s", id), data, nil)
}

func (r *WorkflowsResource) DeleteCronJob(id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/cron/jobs/%s", id), nil, nil)
}

func (r *WorkflowsResource) ToggleCronJob(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/cron/jobs/%s/enable", id), data, nil)
}

func (r *WorkflowsResource) CronJobStatus(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/cron/jobs/%s/status", id), nil, nil)
}

func (r *WorkflowsResource) ListSchedules() (interface{}, error) {
	return r.client.request("GET", "/api/schedules", nil, nil)
}

func (r *WorkflowsResource) CreateSchedule(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/schedules", data, nil)
}

func (r *WorkflowsResource) GetSchedule(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/schedules/%s", id), nil, nil)
}

func (r *WorkflowsResource) UpdateSchedule(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/schedules/%s", id), data, nil)
}

func (r *WorkflowsResource) DeleteSchedule(id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/schedules/%s", id), nil, nil)
}

func (r *WorkflowsResource) RunSchedule(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/schedules/%s/run", id), nil, nil)
}

func (r *WorkflowsResource) ListTriggers(query map[string]string) (interface{}, error) {
	return r.client.request("GET", "/api/triggers", nil, query)
}

func (r *WorkflowsResource) CreateTrigger(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/triggers", data, nil)
}

func (r *WorkflowsResource) GetTrigger(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/triggers/%s", id), nil, nil)
}

func (r *WorkflowsResource) DeleteTrigger(id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/triggers/%s", id), nil, nil)
}

func (r *WorkflowsResource) UpdateTrigger(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PATCH", fmt.Sprintf("/api/triggers/%s", id), data, nil)
}

func (r *WorkflowsResource) ListWorkflows() (interface{}, error) {
	return r.client.request("GET", "/api/workflows", nil, nil)
}

func (r *WorkflowsResource) CreateWorkflow(data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", "/api/workflows", data, nil)
}

func (r *WorkflowsResource) UpdateWorkflow(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("PUT", fmt.Sprintf("/api/workflows/%s", id), data, nil)
}

func (r *WorkflowsResource) DeleteWorkflow(id string) (interface{}, error) {
	return r.client.request("DELETE", fmt.Sprintf("/api/workflows/%s", id), nil, nil)
}

func (r *WorkflowsResource) RunWorkflow(id string, data map[string]interface{}) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/workflows/%s/run", id), data, nil)
}

func (r *WorkflowsResource) ListWorkflowRuns(id string) (interface{}, error) {
	return r.client.request("GET", fmt.Sprintf("/api/workflows/%s/runs", id), nil, nil)
}

func (r *WorkflowsResource) SaveWorkflowAsTemplate(id string) (interface{}, error) {
	return r.client.request("POST", fmt.Sprintf("/api/workflows/%s/save-as-template", id), nil, nil)
}

