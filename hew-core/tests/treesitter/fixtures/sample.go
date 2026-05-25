// Sample Go fixture for TS.4 end-to-end tests.
// Layout: two top-level funcs, one struct + two methods, one closure.

package main

import "fmt"

func AlphaCompute(a, b int) int {
	scale := func(x int) int { return x * 2 }
	return scale(a) + scale(b)
}

func BetaFormat(name string) string {
	return fmt.Sprintf("hello, %s", name)
}

type Widget struct {
	ID int
}

func (w Widget) GammaDescribe() string {
	return fmt.Sprintf("widget-%d", w.ID)
}

func (w Widget) DeltaClone() Widget {
	return Widget{ID: w.ID}
}

func EpsilonDispatch(w Widget) string {
	return w.GammaDescribe()
}
