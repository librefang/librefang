package librefang

import (
	"strings"
	"testing"
)

func TestStreamReportsRequestConstructionError(t *testing.T) {
	client := New("http://daemon.invalid")
	events := client.stream("GET\nInjected: yes", "/events", nil, nil)

	event, ok := <-events
	if !ok {
		t.Fatal("stream closed without reporting the request construction error")
	}
	if got, want := event["status"], 0; got != want {
		t.Fatalf("status = %#v, want %#v", got, want)
	}
	errorText, ok := event["error"].(string)
	if !ok || !strings.Contains(errorText, "new request: net/http: invalid method") {
		t.Fatalf("error = %#v, want invalid-method construction failure", event["error"])
	}
	if _, ok := <-events; ok {
		t.Fatal("stream emitted more than one request construction error")
	}
}
