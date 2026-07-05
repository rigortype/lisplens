;;; lisplens-parinfer.el --- Parinfer for Lisp via the lisplens server -*- lexical-binding: t; -*-

;; Copyright (C) 2026  USAMI Kenta

;; Author: USAMI Kenta <tadsan@zonu.me>
;; URL: https://github.com/rigortype/lisplens
;; Version: 0.1.0
;; Package-Requires: ((emacs "29.1"))
;; Keywords: languages, lisp, tools

;; This program is free software; you can redistribute it and/or modify
;; it under the terms of the Mozilla Public License, v. 2.0.  If a copy of
;; the MPL was not distributed with this file, You can obtain one at
;; https://mozilla.org/MPL/2.0/.

;;; Commentary:

;; A native Emacs front-end for lisplens's parinfer command
;; (`lisplens parinfer`).  It is *not* a fork of parinfer-rust-mode and keeps
;; no compatibility with it: lisplens is a CLI, so this talks to one long-lived
;; `lisplens parinfer --server' process (a line-delimited JSON protocol) shared
;; across every buffer, instead of loading a dynamic module.
;;
;; This file provides the explicit commands `lisplens-parinfer-paren' and
;; `lisplens-parinfer-indent', plus a minor mode `lisplens-parinfer-mode' that
;; binds them.  Firing on every edit (the live experience) is a separate layer.
;;
;;   paren  — parens are the source of truth; indentation is corrected.
;;   indent — indentation is the source of truth; close-parens are inferred.
;;
;; On the server's refusal (unbalanced input, an unterminated string, …) the
;; buffer is left untouched and the diagnostic is echoed.

;;; Code:

(require 'subr-x)

(defgroup lisplens-parinfer nil
  "Parinfer for Lisp, backed by the lisplens server."
  :group 'tools
  :prefix "lisplens-parinfer-")

(defcustom lisplens-parinfer-executable "lisplens"
  "The lisplens executable (found on `exec-path' or an absolute path)."
  :type 'string)

(defcustom lisplens-parinfer-timeout 2.0
  "Seconds to wait for the server to answer one request before giving up."
  :type 'number)

(defcustom lisplens-parinfer-dialect-alist
  '((emacs-lisp-mode . "emacs-lisp")
    (lisp-interaction-mode . "emacs-lisp")
    (lisp-data-mode . "emacs-lisp")
    (lisp-mode . "common-lisp")
    (clojure-mode . "clojure")
    (clojurescript-mode . "clojure")
    (clojurec-mode . "clojure")
    (clojure-ts-mode . "clojure")
    (scheme-mode . "scheme")
    (racket-mode . "racket")
    (fennel-mode . "fennel")
    (janet-mode . "janet")
    (hy-mode . "hy")
    (lfe-mode . "lfe")
    (phel-mode . "phel"))
  "Map a major mode to the lisplens dialect name sent to the server.
When the current major mode (or one of its parents) is absent, no dialect is
sent and the server defaults to Emacs Lisp."
  :type '(alist :key-type symbol :value-type string))

(defcustom lisplens-parinfer-nameless 'auto
  "Whether to enable the Nameless overlay (Emacs Lisp only).
`auto' enables it when `nameless-mode' is active in the buffer; t forces it on,
nil off."
  :type '(choice (const :tag "Follow nameless-mode" auto)
                 (const :tag "On" t)
                 (const :tag "Off" nil)))

;;; Server process ----------------------------------------------------------

(defvar lisplens-parinfer--process nil
  "The shared `lisplens parinfer --server' process, or nil.")

(defvar lisplens-parinfer--output ""
  "Accumulated stdout from the server for the in-flight request.")

(defun lisplens-parinfer--filter (_proc string)
  "Collect STRING from the server into `lisplens-parinfer--output'."
  (setq lisplens-parinfer--output (concat lisplens-parinfer--output string)))

(defun lisplens-parinfer--ensure-process ()
  "Start the shared server process if it is not already running."
  (unless (process-live-p lisplens-parinfer--process)
    (setq lisplens-parinfer--output "")
    (setq lisplens-parinfer--process
          (make-process
           :name "lisplens-parinfer"
           :command (list lisplens-parinfer-executable "parinfer" "--server")
           :connection-type 'pipe
           :coding 'utf-8-unix
           :noquery t
           :filter #'lisplens-parinfer--filter
           :stderr (get-buffer-create " *lisplens-parinfer-stderr*"))))
  lisplens-parinfer--process)

(defun lisplens-parinfer-restart ()
  "Stop the shared server process; the next request starts a fresh one."
  (interactive)
  (when (process-live-p lisplens-parinfer--process)
    (delete-process lisplens-parinfer--process))
  (setq lisplens-parinfer--process nil))

(defun lisplens-parinfer--request (request)
  "Send REQUEST (an alist/plist) to the server and return the parsed answer.
Blocks until one answer line arrives or `lisplens-parinfer-timeout' elapses."
  (lisplens-parinfer--ensure-process)
  (setq lisplens-parinfer--output "")
  (process-send-string lisplens-parinfer--process
                       (concat (json-serialize request) "\n"))
  (let ((deadline (+ (float-time) lisplens-parinfer-timeout)))
    (while (and (not (string-search "\n" lisplens-parinfer--output))
                (< (float-time) deadline)
                (process-live-p lisplens-parinfer--process))
      (accept-process-output lisplens-parinfer--process 0.05)))
  (let ((nl (string-search "\n" lisplens-parinfer--output)))
    (unless nl
      (error "lisplens-parinfer: no response from server"))
    (json-parse-string (substring lisplens-parinfer--output 0 nl)
                       :object-type 'alist :null-object nil :false-object :false)))

;;; Request/answer ----------------------------------------------------------

(defun lisplens-parinfer--dialect ()
  "The lisplens dialect for the current buffer, or nil to let the server decide."
  (seq-some (lambda (cell)
              (and (derived-mode-p (car cell)) (cdr cell)))
            lisplens-parinfer-dialect-alist))

(defun lisplens-parinfer--nameless-p ()
  "Whether to send the Nameless overlay for the current buffer."
  (pcase lisplens-parinfer-nameless
    ('auto (bound-and-true-p nameless-mode))
    (val val)))

(defun lisplens-parinfer--bounds ()
  "The line-aligned (BEG . END) to transform: the active region, else the buffer.
Region bounds are widened to whole lines so the sent text starts at a line
beginning, keeping the cursor line/column arithmetic exact."
  (if (use-region-p)
      (cons (save-excursion (goto-char (region-beginning)) (line-beginning-position))
            (save-excursion (goto-char (region-end)) (line-end-position)))
    (cons (point-min) (point-max))))

(defun lisplens-parinfer--run (mode)
  "Transform the buffer (or region) with MODE (`paren' or `indent')."
  (let* ((bounds (lisplens-parinfer--bounds))
         (beg (car bounds))
         (end (cdr bounds))
         (text (buffer-substring-no-properties beg end))
         (in-scope (<= beg (point) end))
         (dialect (lisplens-parinfer--dialect))
         (request
          (append
           (list :mode (symbol-name mode) :text text)
           (when dialect (list :dialect dialect))
           (when (and (equal dialect "emacs-lisp") (lisplens-parinfer--nameless-p))
             (list :nameless t :name (or (buffer-file-name) "")))
           (when in-scope
             (list :cursorLine (- (line-number-at-pos (point))
                                  (line-number-at-pos beg))
                   :cursorX (- (point) (line-beginning-position))))))
         (answer (lisplens-parinfer--request request)))
    (lisplens-parinfer--apply answer beg end)))

(defun lisplens-parinfer--apply (answer beg end)
  "Apply ANSWER to the region BEG..END, or report its diagnostic on refusal."
  (if (not (eq (alist-get 'success answer) t))
      (let ((err (alist-get 'error answer)))
        (message "lisplens-parinfer: %s (%s)"
                 (or (alist-get 'message err) "refused")
                 (or (alist-get 'name err) "error")))
    (let ((text (alist-get 'text answer))
          (cline (alist-get 'cursorLine answer))
          (cx (alist-get 'cursorX answer)))
      (unless (string= text (buffer-substring-no-properties beg end))
        ;; Minimal-diff replace preserves markers and keeps undo tight.
        (replace-region-contents beg end (lambda () text)))
      (when (and (numberp cline) (numberp cx))
        (goto-char beg)
        (forward-line cline)
        (goto-char (min (+ (point) cx) (line-end-position)))))))

;;; Commands ----------------------------------------------------------------

;;;###autoload
(defun lisplens-parinfer-paren ()
  "Run parinfer paren mode on the buffer (or active region)."
  (interactive)
  (lisplens-parinfer--run 'paren))

;;;###autoload
(defun lisplens-parinfer-indent ()
  "Run parinfer indent mode on the buffer (or active region)."
  (interactive)
  (lisplens-parinfer--run 'indent))

(defvar-keymap lisplens-parinfer-mode-map
  :doc "Keymap for `lisplens-parinfer-mode'."
  "C-c C-p p" #'lisplens-parinfer-paren
  "C-c C-p i" #'lisplens-parinfer-indent)

;;;###autoload
(define-minor-mode lisplens-parinfer-mode
  "Minor mode binding the lisplens parinfer commands.
Provides `lisplens-parinfer-paren' and `lisplens-parinfer-indent'.  Firing on
every edit (the live experience) is layered on top separately."
  :lighter " Parinfer"
  :keymap lisplens-parinfer-mode-map)

(provide 'lisplens-parinfer)
;;; lisplens-parinfer.el ends here
