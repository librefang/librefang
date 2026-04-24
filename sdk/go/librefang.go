/*
LibreFang Go SDK — REST API client for controlling LibreFang remotely.
*/
package librefang

import (
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

	Agents    *AgentResource
	Sessions  *SessionResource
	Workflows *WorkflowResource
	Skills    *SkillResource
	Channels  *ChannelResource
	Tools     *ToolResource
	Models    *ModelResource
	Providers *ProviderResource
	Memory    *MemoryResource
	Triggers  *TriggerResource
	Schedules *ScheduleResource
}

// New creates a new LibreFang client.
func New(baseURL string) *Client {
	baseURL = strings.TrimSuffix(baseURL, "/")
	headers := map[string]string{
		"Content-Type": "application/json",
	}

	c := &Client{
		BaseURL: baseURL,
		Headers: headers,
		HTTP:    &http.Client{},
	}

	c.Agents = &AgentResource{client: c}
	c.Sessions = &SessionResource{client: c}
	c.Workflows = &WorkflowResource{client: c}
	c.Skills = &SkillResource{client: c}
	c.Channels = &ChannelResource{client: c}
	c.Tools = &ToolResource{client: c}
	c.Models = &ModelResource{client: c}
	c.Providers = &ProviderResource{client: c}
	c.Memory = &MemoryResource{client: c}
	c.Triggers = &TriggerResource{client: c}
	c.Schedules = &ScheduleResource{client: c}

	return c
}

func (c *Client) doRequest(method, path string, body interface{}) (interface{}, error) {
	url := c.BaseURL + path

	var bodyBytes []byte
	if body != nil {
		b, err := json.Marshal(body)
		if err != nil {
			return nil, fmt.Errorf("marshal request body: %w", err)
		}
		bodyBytes = b
	}

	req, err := http.NewRequest(method, url, bytes.NewReader(bodyBytes))
	if err != nil {
		return nil, fmt.Errorf("create request: %w", err)
	}

	for k, v := range c.Headers {
		req.Header.Set(k, v)
	}

	resp, err := c.HTTP.Do(req)
	if err != nil {
		return nil, fmt.Errorf("request failed: %w", err)
	}
	defer resp.Body.Close()

	respBody, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, fmt.Errorf("read response: %w", err)
	}

	if resp.StatusCode >= 400 {
		return nil, &LibreFangError{
			Message: string(respBody),
			Status:  resp.StatusCode,
			Body:    string(respBody),
		}
	}

	// Try array first, then object
	var arr []json.RawMessage
	if err := json.Unmarshal(respBody, &arr); err == nil {
		return arr, nil
	}

	var result map[string]interface{}
	if err := json.Unmarshal(respBody, &result); err != nil {
		return nil, fmt.Errorf("unmarshal response: %w", err)
	}
	return result, nil
}

func (c *Client) doStream(method, path string, body interface{}) <-chan map[string]interface{} {
	ch := make(chan map[string]interface{})

	go func() {
		defer close(ch)

		url := c.BaseURL + path
		var bodyBytes []byte
		if body != nil {
			b, _ := json.Marshal(body)
			bodyBytes = b
		}

		req, _ := http.NewRequest(method, url, bytes.NewReader(bodyBytes))
		for k, v := range c.Headers {
			req.Header.Set(k, v)
		}
		req.Header.Set("Accept", "text/event-stream")

		resp, err := c.HTTP.Do(req)
		if err != nil {
			ch <- map[string]interface{}{"error": err.Error()}
			return
		}
		defer resp.Body.Close()

		if resp.StatusCode >= 400 {
			body, _ := io.ReadAll(resp.Body)
			ch <- map[string]interface{}{"error": fmt.Sprintf("HTTP %d: %s", resp.StatusCode, string(body))}
			return
		}

		buf := make([]byte, 4096)
		for {
			n, err := resp.Body.Read(buf)
			if n > 0 {
				text := string(buf[:n])
				lines := strings.Split(text, "\n")
				for _, line := range lines {
					line = strings.TrimSpace(line)
					if strings.HasPrefix(line, "data: ") {
						data := strings.TrimPrefix(line, "data: ")
						if data == "[DONE]" {
							return
						}
						var event map[string]interface{}
						if err := json.Unmarshal([]byte(data), &event); err != nil {
							ch <- map[string]interface{}{"raw": data}
						} else {
							ch <- event
						}
					}
				}
			}
			if err != nil {
				break
			}
		}
	}()

	return ch
}

// Helper to convert interface{} to map
func toMap(v interface{}) map[string]interface{} {
	if m, ok := v.(map[string]interface{}); ok {
		return m
	}
	return map[string]interface{}{}
}

// Helper to convert interface{} to slice of maps
func toSlice(v interface{}) []map[string]interface{} {
	if v == nil {
		return nil
	}
	if arr, ok := v.([]json.RawMessage); ok {
		result := make([]map[string]interface{}, len(arr))
		for i, raw := range arr {
			json.Unmarshal(raw, &result[i])
		}
		return result
	}
	if arr, ok := v.([]interface{}); ok {
		result := make([]map[string]interface{}, len(arr))
		for i, a := range arr {
			if m, ok := a.(map[string]interface{}); ok {
				result[i] = m
			}
		}
		return result
	}
	return []map[string]interface{}{}
}

func getSlice(data interface{}, key string) []map[string]interface{} {
	if m, ok := data.(map[string]interface{}); ok {
		return toSlice(m[key])
	}
	return toSlice(data)
}

// --- Client Methods ---

func (c *Client) Health() (map[string]interface{}, error) {
	resp, err := c.doRequest("GET", "/api/health", nil)
	return toMap(resp), err
}

func (c *Client) HealthDetail() (map[string]interface{}, error) {
	resp, err := c.doRequest("GET", "/api/health/detail", nil)
	return toMap(resp), err
}

func (c *Client) Status() (map[string]interface{}, error) {
	resp, err := c.doRequest("GET", "/api/status", nil)
	return toMap(resp), err
}

func (c *Client) Version() (map[string]interface{}, error) {
	resp, err := c.doRequest("GET", "/api/version", nil)
	return toMap(resp), err
}

func (c *Client) Metrics() (string, error) {
	resp, _ := c.doRequest("GET", "/api/metrics", nil)
	if m, ok := resp.(map[string]interface{}); ok {
		if text, ok := m["text"].(string); ok {
			return text, nil
		}
	}
	return "", nil
}

func (c *Client) Usage() (map[string]interface{}, error) {
	resp, err := c.doRequest("GET", "/api/usage", nil)
	return toMap(resp), err
}

func (c *Client) Config() (map[string]interface{}, error) {
	resp, err := c.doRequest("GET", "/api/config", nil)
	return toMap(resp), err
}

// --- Agent Resource ---

type AgentResource struct{ client *Client }

func (r *AgentResource) List() ([]map[string]interface{}, error) {
	data, _ := r.client.doRequest("GET", "/api/agents", nil)
	return toSlice(data), nil
}

func (r *AgentResource) Get(id string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("GET", fmt.Sprintf("/api/agents/%s", id), nil)
	return toMap(resp), err
}

func (r *AgentResource) Create(params map[string]interface{}) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("POST", "/api/agents", params)
	return toMap(resp), err
}

func (r *AgentResource) Delete(id string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("DELETE", fmt.Sprintf("/api/agents/%s", id), nil)
	return toMap(resp), err
}

func (r *AgentResource) Stop(id string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("POST", fmt.Sprintf("/api/agents/%s/stop", id), nil)
	return toMap(resp), err
}

func (r *AgentResource) Clone(id string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("POST", fmt.Sprintf("/api/agents/%s/clone", id), nil)
	return toMap(resp), err
}

func (r *AgentResource) Update(id string, data map[string]interface{}) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("PUT", fmt.Sprintf("/api/agents/%s/update", id), data)
	return toMap(resp), err
}

func (r *AgentResource) SetMode(id, mode string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("PUT", fmt.Sprintf("/api/agents/%s/mode", id), map[string]interface{}{"mode": mode})
	return toMap(resp), err
}

func (r *AgentResource) SetModel(id, model string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("PUT", fmt.Sprintf("/api/agents/%s/model", id), map[string]interface{}{"model": model})
	return toMap(resp), err
}

func (r *AgentResource) Message(id, text string, opts ...map[string]interface{}) (map[string]interface{}, error) {
	body := map[string]interface{}{"message": text}
	if len(opts) > 0 {
		for k, v := range opts[0] {
			body[k] = v
		}
	}
	resp, err := r.client.doRequest("POST", fmt.Sprintf("/api/agents/%s/message", id), body)
	return toMap(resp), err
}

func (r *AgentResource) Stream(id, text string, opts ...map[string]interface{}) <-chan map[string]interface{} {
	body := map[string]interface{}{"message": text}
	if len(opts) > 0 {
		for k, v := range opts[0] {
			body[k] = v
		}
	}
	return r.client.doStream("POST", fmt.Sprintf("/api/agents/%s/message/stream", id), body)
}

func (r *AgentResource) Session(id string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("GET", fmt.Sprintf("/api/agents/%s/session", id), nil)
	return toMap(resp), err
}

func (r *AgentResource) ResetSession(id string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("POST", fmt.Sprintf("/api/agents/%s/session/reset", id), nil)
	return toMap(resp), err
}

func (r *AgentResource) ListSessions(id string) ([]map[string]interface{}, error) {
	data, _ := r.client.doRequest("GET", fmt.Sprintf("/api/agents/%s/sessions", id), nil)
	return getSlice(data, "sessions"), nil
}

func (r *AgentResource) CreateSession(id, label string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("POST", fmt.Sprintf("/api/agents/%s/sessions", id), map[string]interface{}{"label": label})
	return toMap(resp), err
}

func (r *AgentResource) SwitchSession(id, sessionID string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("POST", fmt.Sprintf("/api/agents/%s/sessions/%s/switch", id, sessionID), nil)
	return toMap(resp), err
}

func (r *AgentResource) GetSkills(id string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("GET", fmt.Sprintf("/api/agents/%s/skills", id), nil)
	return toMap(resp), err
}

func (r *AgentResource) SetSkills(id string, skills []string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("PUT", fmt.Sprintf("/api/agents/%s/skills", id), skills)
	return toMap(resp), err
}

func (r *AgentResource) SetIdentity(id string, identity map[string]interface{}) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("PATCH", fmt.Sprintf("/api/agents/%s/identity", id), identity)
	return toMap(resp), err
}

func (r *AgentResource) PatchConfig(id string, config map[string]interface{}) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("PATCH", fmt.Sprintf("/api/agents/%s/config", id), config)
	return toMap(resp), err
}

// --- Session Resource ---

type SessionResource struct{ client *Client }

func (r *SessionResource) List() ([]map[string]interface{}, error) {
	data, _ := r.client.doRequest("GET", "/api/sessions", nil)
	return getSlice(data, "sessions"), nil
}

func (r *SessionResource) Delete(id string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("DELETE", fmt.Sprintf("/api/sessions/%s", id), nil)
	return toMap(resp), err
}

func (r *SessionResource) SetLabel(id, label string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("PUT", fmt.Sprintf("/api/sessions/%s/label", id), map[string]interface{}{"label": label})
	return toMap(resp), err
}

// --- Workflow Resource ---

type WorkflowResource struct{ client *Client }

func (r *WorkflowResource) List() ([]map[string]interface{}, error) {
	data, _ := r.client.doRequest("GET", "/api/workflows", nil)
	return getSlice(data, "workflows"), nil
}

func (r *WorkflowResource) Create(workflow map[string]interface{}) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("POST", "/api/workflows", workflow)
	return toMap(resp), err
}

func (r *WorkflowResource) Run(id string, input interface{}) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("POST", fmt.Sprintf("/api/workflows/%s/run", id), input)
	return toMap(resp), err
}

func (r *WorkflowResource) Runs(id string) ([]map[string]interface{}, error) {
	data, _ := r.client.doRequest("GET", fmt.Sprintf("/api/workflows/%s/runs", id), nil)
	return getSlice(data, "runs"), nil
}

// --- Skill Resource ---

type SkillResource struct{ client *Client }

func (r *SkillResource) List() ([]map[string]interface{}, error) {
	data, _ := r.client.doRequest("GET", "/api/skills", nil)
	return getSlice(data, "skills"), nil
}

func (r *SkillResource) Install(skill map[string]interface{}) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("POST", "/api/skills/install", skill)
	return toMap(resp), err
}

func (r *SkillResource) Uninstall(skill map[string]interface{}) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("POST", "/api/skills/uninstall", skill)
	return toMap(resp), err
}

func (r *SkillResource) Search(query string) ([]map[string]interface{}, error) {
	data, _ := r.client.doRequest("GET", fmt.Sprintf("/api/marketplace/search?q=%s", query), nil)
	return getSlice(data, "results"), nil
}

// --- Channel Resource ---

type ChannelResource struct{ client *Client }

func (r *ChannelResource) List() ([]map[string]interface{}, error) {
	data, _ := r.client.doRequest("GET", "/api/channels", nil)
	return getSlice(data, "channels"), nil
}

func (r *ChannelResource) Configure(name string, config map[string]interface{}) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("POST", fmt.Sprintf("/api/channels/%s/configure", name), config)
	return toMap(resp), err
}

func (r *ChannelResource) Remove(name string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("DELETE", fmt.Sprintf("/api/channels/%s/configure", name), nil)
	return toMap(resp), err
}

func (r *ChannelResource) Test(name string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("POST", fmt.Sprintf("/api/channels/%s/test", name), nil)
	return toMap(resp), err
}

// --- Tool Resource ---

type ToolResource struct{ client *Client }

func (r *ToolResource) List() ([]map[string]interface{}, error) {
	data, _ := r.client.doRequest("GET", "/api/tools", nil)
	return getSlice(data, "tools"), nil
}

func (r *ToolResource) Get(name string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("GET", fmt.Sprintf("/api/tools/%s", name), nil)
	return toMap(resp), err
}

// Invoke calls /api/tools/{name}/invoke. Pass a non-empty agentID for
// approval-gated tools so the server can resolve the deferred execution
// to a real agent; pass "" for tools that do not require approval.
func (r *ToolResource) Invoke(name string, input map[string]interface{}, agentID string) (map[string]interface{}, error) {
	path := fmt.Sprintf("/api/tools/%s/invoke", url.PathEscape(name))
	if agentID != "" {
		path += "?agent_id=" + url.QueryEscape(agentID)
	}
	resp, err := r.client.doRequest("POST", path, input)
	return toMap(resp), err
}

// --- Model Resource ---

type ModelResource struct{ client *Client }

func (r *ModelResource) List() ([]map[string]interface{}, error) {
	data, _ := r.client.doRequest("GET", "/api/models", nil)
	return getSlice(data, "models"), nil
}

func (r *ModelResource) Get(id string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("GET", fmt.Sprintf("/api/models/%s", id), nil)
	return toMap(resp), err
}

func (r *ModelResource) Aliases() (map[string]interface{}, error) {
	resp, err := r.client.doRequest("GET", "/api/models/aliases", nil)
	return toMap(resp), err
}

// --- Provider Resource ---

type ProviderResource struct{ client *Client }

func (r *ProviderResource) List() ([]map[string]interface{}, error) {
	data, _ := r.client.doRequest("GET", "/api/providers", nil)
	return getSlice(data, "providers"), nil
}

func (r *ProviderResource) SetKey(name, key string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("POST", fmt.Sprintf("/api/providers/%s/key", name), map[string]interface{}{"key": key})
	return toMap(resp), err
}

func (r *ProviderResource) DeleteKey(name string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("DELETE", fmt.Sprintf("/api/providers/%s/key", name), nil)
	return toMap(resp), err
}

func (r *ProviderResource) Test(name string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("POST", fmt.Sprintf("/api/providers/%s/test", name), nil)
	return toMap(resp), err
}

// --- Memory Resource ---

type MemoryResource struct{ client *Client }

func (r *MemoryResource) GetAll(agentID string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("GET", fmt.Sprintf("/api/memory/agents/%s/kv", agentID), nil)
	return toMap(resp), err
}

func (r *MemoryResource) Get(agentID, key string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("GET", fmt.Sprintf("/api/memory/agents/%s/kv/%s", agentID, key), nil)
	return toMap(resp), err
}

func (r *MemoryResource) Set(agentID, key string, value interface{}) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("PUT", fmt.Sprintf("/api/memory/agents/%s/kv/%s", agentID, key), map[string]interface{}{"value": value})
	return toMap(resp), err
}

func (r *MemoryResource) Delete(agentID, key string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("DELETE", fmt.Sprintf("/api/memory/agents/%s/kv/%s", agentID, key), nil)
	return toMap(resp), err
}

// --- Trigger Resource ---

type TriggerResource struct{ client *Client }

func (r *TriggerResource) List() ([]map[string]interface{}, error) {
	data, _ := r.client.doRequest("GET", "/api/triggers", nil)
	return getSlice(data, "triggers"), nil
}

func (r *TriggerResource) Create(trigger map[string]interface{}) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("POST", "/api/triggers", trigger)
	return toMap(resp), err
}

func (r *TriggerResource) Update(id string, trigger map[string]interface{}) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("PUT", fmt.Sprintf("/api/triggers/%s", id), trigger)
	return toMap(resp), err
}

func (r *TriggerResource) Delete(id string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("DELETE", fmt.Sprintf("/api/triggers/%s", id), nil)
	return toMap(resp), err
}

// --- Schedule Resource ---

type ScheduleResource struct{ client *Client }

func (r *ScheduleResource) List() ([]map[string]interface{}, error) {
	data, _ := r.client.doRequest("GET", "/api/schedules", nil)
	return getSlice(data, "schedules"), nil
}

func (r *ScheduleResource) Create(schedule map[string]interface{}) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("POST", "/api/schedules", schedule)
	return toMap(resp), err
}

func (r *ScheduleResource) Update(id string, schedule map[string]interface{}) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("PUT", fmt.Sprintf("/api/schedules/%s", id), schedule)
	return toMap(resp), err
}

func (r *ScheduleResource) Delete(id string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("DELETE", fmt.Sprintf("/api/schedules/%s", id), nil)
	return toMap(resp), err
}

func (r *ScheduleResource) Run(id string) (map[string]interface{}, error) {
	resp, err := r.client.doRequest("POST", fmt.Sprintf("/api/schedules/%s/run", id), nil)
	return toMap(resp), err
}
